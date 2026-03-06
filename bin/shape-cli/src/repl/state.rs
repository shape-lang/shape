//! REPL state management using Cells

use crate::repl::cells::{Cell, CellType};
use shape_wire::ValueEnvelope;
use std::collections::HashMap;
use tui_textarea::TextArea;

/// State snapshot for time-travel
#[derive(Debug, Clone)]
pub struct StateSnapshot<'a> {
    pub cells: Vec<Cell<'a>>,
}

/// REPL state with scrolling cells
pub struct ReplState<'a> {
    /// List of cells (inputs, outputs, errors)
    pub cells: Vec<Cell<'a>>,

    /// ID counter for cells
    next_cell_id: usize,

    /// Index of the currently focused cell (if any, for inspection)
    pub focused_cell_index: Option<usize>,

    /// Undo history
    snapshots: Vec<StateSnapshot<'a>>,

    /// Redo stack
    redo_stack: Vec<StateSnapshot<'a>>,
}

impl<'a> ReplState<'a> {
    pub fn new() -> Self {
        let mut state = Self {
            cells: Vec::new(),
            next_cell_id: 0,
            focused_cell_index: None,
            snapshots: Vec::new(),
            redo_stack: Vec::new(),
        };
        // Initialize with one active input cell
        state.add_active_input();
        state
    }

    fn next_id(&mut self) -> usize {
        let id = self.next_cell_id;
        self.next_cell_id += 1;
        id
    }

    fn save_snapshot(&mut self) {
        self.redo_stack.clear();
        self.snapshots.push(StateSnapshot {
            cells: self.cells.clone(),
        });
    }

    /// Add a new active input cell at the end
    pub fn add_active_input(&mut self) {
        let id = self.next_id();
        self.cells.push(Cell::active_input(id));
    }

    /// Insert a new active input cell at a specific index
    pub fn insert_active_input_at(&mut self, index: usize) -> usize {
        let id = self.next_id();
        let insert_at = index.min(self.cells.len());
        self.cells.insert(insert_at, Cell::active_input(id));
        insert_at
    }

    /// Delete a cell at index
    pub fn delete_cell(&mut self, index: usize) {
        if index < self.cells.len() {
            self.save_snapshot();
            self.cells.remove(index);
        }
    }

    /// Enable edit mode for a cell
    pub fn enter_edit_mode(&mut self, index: usize) -> bool {
        if let Some(cell) = self.cells.get_mut(index) {
            if cell.kind == CellType::Input {
                if cell.input_editor.is_some() {
                    cell.focused = true;
                    return true;
                }
                let lines: Vec<String> = cell.content.lines().map(|s| s.to_string()).collect();
                let mut textarea = if lines.is_empty() {
                    TextArea::default()
                } else {
                    TextArea::new(lines)
                };

                textarea.set_cursor_line_style(ratatui::style::Style::default());
                textarea.set_placeholder_text("Enter code...");

                cell.input_editor = Some(textarea);
                cell.focused = true;
                cell.executed = false;
                return true;
            }
        }
        false
    }

    /// Commit the currently focused input cell
    /// Returns (index, content) if found
    pub fn commit_focused_input(&mut self) -> Option<(usize, String)> {
        let focused_idx = self
            .cells
            .iter()
            .position(|c| c.focused && c.input_editor.is_some())?;

        let content = if let Some(cell) = self.cells.get(focused_idx) {
            cell.input_editor.as_ref().map(|e| e.lines().join("\n"))
        } else {
            None
        };

        if let Some(content) = content {
            self.save_snapshot();
            if let Some(cell) = self.cells.get_mut(focused_idx) {
                cell.input_editor = None;
                cell.content = content.clone();
                cell.focused = false;
            }
            Some((focused_idx, content))
        } else {
            None
        }
    }

    /// Commit the input cell at specific index
    pub fn commit_cell(&mut self, index: usize) -> Option<String> {
        // First get content if it's an input cell
        let content_to_save = if let Some(cell) = self.cells.get(index) {
            if cell.kind == CellType::Input {
                if let Some(editor) = &cell.input_editor {
                    Some(editor.lines().join("\n"))
                } else {
                    Some(cell.content.clone())
                }
            } else {
                None
            }
        } else {
            None
        };

        // Then apply updates
        if let Some(content) = content_to_save {
            // Only update if there was an editor (otherwise content is already in cell.content)
            // But we might want to ensure focused=false
            let has_editor = self.cells[index].input_editor.is_some();

            if has_editor {
                self.save_snapshot();
                if let Some(cell) = self.cells.get_mut(index) {
                    cell.input_editor = None;
                    cell.content = content.clone();
                    cell.focused = false;
                }
            } else if self.cells[index].focused {
                // Just unfocus
                if let Some(cell) = self.cells.get_mut(index) {
                    cell.focused = false;
                }
            }

            Some(content)
        } else {
            None
        }
    }

    // Deprecated wrapper for backward compatibility during refactor
    pub fn commit_current_input(&mut self) -> Option<String> {
        self.commit_focused_input().map(|(_, c)| c)
    }

    /// Add or update a print output after a specific input index
    pub fn update_print_for(&mut self, input_index: usize, text: String) {
        let output_index = input_index + 1;
        let id = self.next_id();
        let mut cell = Cell::new_output(id, serde_json::Value::Null, text, None);
        cell.tree_state = None; // No tree view for text output

        let replace = if let Some(next) = self.cells.get(output_index) {
            next.kind == CellType::Output || next.kind == CellType::Error
        } else {
            false
        };

        if replace {
            self.cells[output_index] = cell;
        } else {
            self.cells.insert(output_index, cell);
        }
    }

    /// Add or update output after a specific input index
    pub fn update_output_for(
        &mut self,
        input_index: usize,
        value: serde_json::Value,
        envelope: ValueEnvelope,
        engine: &mut shape_runtime::engine::ShapeEngine,
    ) {
        let output_index = input_index + 1;
        let id = self.next_id();
        let default_format = envelope.default_format().to_string();

        let formatted = if let Some(formatted) = Self::format_with_engine(
            engine,
            &envelope.value,
            &envelope.type_info.name,
            &default_format,
        ) {
            formatted
        } else {
            envelope
                .format(&default_format, &HashMap::new())
                .unwrap_or_else(|_| {
                    if value.is_number() {
                        value.to_string()
                    } else if let Some(s) = value.as_str() {
                        s.to_string()
                    } else {
                        serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
                    }
                })
        };

        let new_output = Cell::new_output(id, value, formatted, Some(envelope));

        // Check if next cell is already an output/error that we should replace
        let replace = if let Some(next) = self.cells.get(output_index) {
            next.kind == CellType::Output || next.kind == CellType::Error
        } else {
            false
        };

        if replace {
            self.cells[output_index] = new_output;
        } else {
            self.cells.insert(output_index, new_output);
        }
    }

    /// Add an error cell (or update)
    pub fn update_error_for(&mut self, input_index: usize, error: String) {
        let output_index = input_index + 1;
        let id = self.next_id();
        let new_error = Cell::new_error(id, error);

        let replace = if let Some(next) = self.cells.get(output_index) {
            next.kind == CellType::Output || next.kind == CellType::Error
        } else {
            false
        };

        if replace {
            self.cells[output_index] = new_error;
        } else {
            self.cells.insert(output_index, new_error);
        }
    }

    // Legacy add_output (appends)
    pub fn add_output(
        &mut self,
        value: serde_json::Value,
        envelope: ValueEnvelope,
        engine: &mut shape_runtime::engine::ShapeEngine,
    ) {
        // Just calling update_output_for with last index is risky if last is not the input.
        // But for legacy compatibility (if used), we assume append.
        self.update_output_for(self.cells.len().saturating_sub(1), value, envelope, engine);
    }

    /// Add an error cell (legacy)
    pub fn add_error(&mut self, error: String) {
        self.update_error_for(self.cells.len().saturating_sub(1), error);
    }

    /// Remove output/error cell following an input if it exists
    pub fn remove_output_for(&mut self, input_index: usize) {
        let output_index = input_index + 1;
        if let Some(next) = self.cells.get(output_index) {
            if next.kind == CellType::Output || next.kind == CellType::Error {
                self.cells.remove(output_index);
            }
        }
    }

    /// Clear all cells and reset
    pub fn clear(&mut self) {
        self.save_snapshot();
        self.cells.clear();
        self.add_active_input();
    }

    /// Invalidate outputs and snapshots after a given input index.
    ///
    /// Keeps input cells (so users can re-run them) but removes any outputs/errors
    /// and clears checkpoint snapshots for inputs after the given index.
    pub fn invalidate_after(&mut self, index: usize) {
        if self.cells.is_empty() || index + 1 >= self.cells.len() {
            return;
        }
        for i in (index + 1..self.cells.len()).rev() {
            if self.cells[i].kind == CellType::Input {
                self.cells[i].execution_time = None;
                self.cells[i].snapshot_id = None;
            } else {
                self.cells.remove(i);
            }
        }
    }

    /// Undo last action
    pub fn undo(&mut self) -> bool {
        if let Some(snapshot) = self.snapshots.pop() {
            // Save current state to redo
            self.redo_stack.push(StateSnapshot {
                cells: self.cells.clone(),
            });
            // Restore snapshot
            self.cells = snapshot.cells;
            return true;
        }
        false
    }

    /// Redo last undo
    pub fn redo(&mut self) -> bool {
        if let Some(snapshot) = self.redo_stack.pop() {
            // Save current to undo
            self.snapshots.push(StateSnapshot {
                cells: self.cells.clone(),
            });
            // Restore snapshot
            self.cells = snapshot.cells;
            return true;
        }
        false
    }

    /// Format a wire value using Shape runtime
    fn format_with_engine(
        engine: &mut shape_runtime::engine::ShapeEngine,
        wire_value: &shape_wire::WireValue,
        type_name: &str,
        format_name: &str,
    ) -> Option<String> {
        match wire_value {
            shape_wire::WireValue::Number(n) => engine
                .format_value_string(*n, type_name, Some(format_name), &HashMap::new())
                .ok(),
            _ => None,
        }
    }
}

impl<'a> Default for ReplState<'a> {
    fn default() -> Self {
        Self::new()
    }
}
