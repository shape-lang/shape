//! Event handling for the REPL
//!
//! This module handles all keyboard input and modal editing logic.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::repl::ReplApp;

/// REPL operation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplMode {
    /// Normal mode - navigation and commands
    Normal,
    /// Insert mode - typing input
    Insert,
    /// Command mode - after pressing ':'
    Command,
    /// Output inspection mode - navigating results
    OutputInspect,
}

impl ReplMode {
    pub fn display_name(&self) -> &'static str {
        match self {
            ReplMode::Normal => "NORMAL",
            ReplMode::Insert => "INSERT",
            ReplMode::Command => "COMMAND",
            ReplMode::OutputInspect => "INSPECT",
        }
    }
}

impl<'a> ReplApp<'a> {
    pub(super) async fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        match self.mode {
            ReplMode::Insert => self.handle_insert_mode(key).await,
            ReplMode::Normal => self.handle_normal_mode(key).await,
            ReplMode::Command => self.handle_command_mode(key).await,
            ReplMode::OutputInspect => self.handle_inspect_mode(key).await,
        }
    }

    async fn handle_insert_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                if let Some((idx, _)) = self.state.commit_focused_input() {
                    self.selected_cell_index = Some(idx);
                }
                self.mode = ReplMode::Normal;
            }
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    || key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    // Execute and move to a new cell below
                    self.execute_focused_input(true).await?;
                } else {
                    // Newline in FOCUSED editor
                    if let Some(cell) = self.state.cells.iter_mut().find(|c| c.focused) {
                        if let Some(editor) = &mut cell.input_editor {
                            editor.insert_newline();
                        }
                    }
                }
            }
            _ => {
                // Pass to FOCUSED editor
                if let Some(cell) = self.state.cells.iter_mut().find(|c| c.focused) {
                    if let Some(editor) = &mut cell.input_editor {
                        use tui_textarea::Input;
                        editor.input(Input::from(key));
                    }
                }
            }
        }
        Ok(())
    }

    async fn handle_normal_mode(&mut self, key: KeyEvent) -> Result<()> {
        // Clear pending keys if not continuing a sequence
        if !self.pending_keys.is_empty()
            && key.code != KeyCode::Char('d')
            && key.code != KeyCode::Char('g')
        {
            self.pending_keys.clear();
        }

        match key.code {
            KeyCode::Char('i') => {
                if let Some(idx) = self.selected_cell_index {
                    // Try to edit selected cell
                    if self.state.enter_edit_mode(idx) {
                        self.mode = ReplMode::Insert;
                    } else if let Some(last_idx) = self.state.cells.len().checked_sub(1) {
                        // Fallback to last active input if selection is not editable
                        if self.state.cells[last_idx].kind == crate::repl::CellType::Input {
                            self.selected_cell_index = Some(last_idx);
                            self.state.enter_edit_mode(last_idx);
                            self.mode = ReplMode::Insert;
                        }
                    }
                } else {
                    // No selection, focus last
                    if let Some(last_idx) = self.state.cells.len().checked_sub(1) {
                        self.selected_cell_index = Some(last_idx);
                        self.state.enter_edit_mode(last_idx);
                        self.mode = ReplMode::Insert;
                    }
                }
            }
            KeyCode::Char('a') => {
                if let Some(idx) = self.selected_cell_index {
                    if self.state.enter_edit_mode(idx) {
                        self.mode = ReplMode::Insert;
                        if let Some(cell) = self.state.cells.get_mut(idx) {
                            if let Some(editor) = &mut cell.input_editor {
                                editor.move_cursor(tui_textarea::CursorMove::End);
                            }
                        }
                    }
                }
            }
            KeyCode::Char('A') => {
                // Append at end of last line (like vim A)
                if let Some(idx) = self.selected_cell_index {
                    if self.state.enter_edit_mode(idx) {
                        self.mode = ReplMode::Insert;
                        if let Some(cell) = self.state.cells.get_mut(idx) {
                            if let Some(editor) = &mut cell.input_editor {
                                editor.move_cursor(tui_textarea::CursorMove::Bottom);
                                editor.move_cursor(tui_textarea::CursorMove::End);
                            }
                        }
                    }
                }
            }
            KeyCode::Char('I') => {
                // Insert at beginning (like vim I)
                if let Some(idx) = self.selected_cell_index {
                    if self.state.enter_edit_mode(idx) {
                        self.mode = ReplMode::Insert;
                        if let Some(cell) = self.state.cells.get_mut(idx) {
                            if let Some(editor) = &mut cell.input_editor {
                                editor.move_cursor(tui_textarea::CursorMove::Top);
                                editor.move_cursor(tui_textarea::CursorMove::Head);
                            }
                        }
                    }
                }
            }
            KeyCode::Char('G') => {
                // Go to last cell
                if !self.state.cells.is_empty() {
                    self.selected_cell_index = Some(self.state.cells.len() - 1);
                }
            }
            KeyCode::Char('g') => {
                if self.pending_keys == "g" {
                    // gg - go to first cell
                    self.selected_cell_index = Some(0);
                    self.pending_keys.clear();
                } else {
                    self.pending_keys = "g".to_string();
                }
            }
            KeyCode::Char('x') => {
                // Execute cell in-place (without creating new input)
                if let Some(idx) = self.selected_cell_index {
                    if let Some(cell) = self.state.cells.get(idx) {
                        if cell.kind == crate::repl::CellType::Input {
                            self.execute_cell(idx, false).await?;
                        }
                    }
                }
            }
            KeyCode::Char('r') => {
                // Rerun all cells from top
                self.set_status("Rerunning all cells...");
                self.rerun_all_cells().await?;
                self.set_status("All cells re-executed");
            }
            KeyCode::Char('o') => {
                // Execute current cell and move next
                if let Some(idx) = self.selected_cell_index {
                    if let Some(cell) = self.state.cells.get(idx) {
                        if cell.kind == crate::repl::CellType::Input {
                            self.execute_cell(idx, true).await?;
                        }
                    }
                }
            }
            KeyCode::Char('d') => {
                if self.pending_keys == "d" {
                    // dd - delete cell
                    if let Some(idx) = self.selected_cell_index {
                        self.state.delete_cell(idx);
                        self.set_status("Deleted cell");
                        // Adjust selection
                        if self.state.cells.is_empty() {
                            self.state.add_active_input();
                            self.selected_cell_index = Some(0);
                        } else if idx >= self.state.cells.len() {
                            self.selected_cell_index = self.state.cells.len().checked_sub(1);
                        }
                    }
                    self.pending_keys.clear();
                } else {
                    self.pending_keys = "d".to_string();
                }
            }
            KeyCode::Char(':') => {
                self.mode = ReplMode::Command;
                self.command_input.clear();
            }
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(selected) = self.selected_cell_index {
                    if selected + 1 < self.state.cells.len() {
                        self.selected_cell_index = Some(selected + 1);
                    }
                } else if !self.state.cells.is_empty() {
                    // Default to latest cell
                    self.selected_cell_index = Some(self.state.cells.len().saturating_sub(1));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(selected) = self.selected_cell_index {
                    if selected > 0 {
                        self.selected_cell_index = Some(selected - 1);
                    }
                } else if !self.state.cells.is_empty() {
                    // Default to latest cell
                    self.selected_cell_index = Some(self.state.cells.len().saturating_sub(1));
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if let Some(idx) = self.selected_cell_index {
                    if let Some(cell) = self.state.cells.get_mut(idx) {
                        cell.collapsed = true;
                    }
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if let Some(idx) = self.selected_cell_index {
                    if let Some(cell) = self.state.cells.get_mut(idx) {
                        cell.collapsed = false;
                    }
                }
            }
            KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.state.undo() {
                    self.set_status("Undone");
                } else {
                    self.set_status("Nothing to undo");
                }
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.state.redo() {
                    self.set_status("Redone");
                } else {
                    self.set_status("Nothing to redo");
                }
            }
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
                if let Some(idx) = self.selected_cell_index {
                    let cell_kind = self.state.cells.get(idx).map(|c| c.kind.clone());
                    if matches!(cell_kind, Some(crate::repl::CellType::Input)) {
                        // Execute the input cell
                        self.execute_cell(idx, true).await?;
                    } else if matches!(cell_kind, Some(crate::repl::CellType::Output)) {
                        if let Some(cell) = self.state.cells.get_mut(idx) {
                            cell.collapsed = false; // Auto-expand
                        }
                        self.mode = ReplMode::OutputInspect;
                        self.set_status("Inspect Mode (Esc to exit)");
                    }
                } else {
                    // No selection: insert at end
                    if let Some(last_idx) = self.state.cells.len().checked_sub(1) {
                        let new_idx = self.state.insert_active_input_at(last_idx + 1);
                        self.selected_cell_index = Some(new_idx);
                        self.mode = ReplMode::Insert;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_command_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = ReplMode::Normal;
                self.command_input.clear();
            }
            KeyCode::Enter => {
                let cmd = self.command_input.clone();
                self.command_input.clear();
                self.mode = ReplMode::Normal;
                self.execute_command(&cmd).await?;
            }
            KeyCode::Backspace => {
                self.command_input.pop();
                if self.command_input.is_empty() {
                    self.mode = ReplMode::Normal;
                }
            }
            KeyCode::Char(c) => {
                self.command_input.push(c);
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_inspect_mode(&mut self, key: KeyEvent) -> Result<()> {
        let idx = if let Some(idx) = self.selected_cell_index {
            idx
        } else {
            return Ok(());
        };

        if idx >= self.state.cells.len() {
            return Ok(());
        }
        let cell = &mut self.state.cells[idx];

        if let Some(tree_state) = &mut cell.tree_state {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.mode = ReplMode::Normal,
                KeyCode::Char('j') | KeyCode::Down => tree_state.selected += 1,
                KeyCode::Char('k') | KeyCode::Up => {
                    if tree_state.selected > 0 {
                        tree_state.selected -= 1;
                    }
                }
                KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => {
                    if let Some(value) = &cell.value {
                        let path = crate::repl::widgets::get_path_at_index(
                            value,
                            tree_state,
                            tree_state.selected,
                        );
                        if let Some(path) = path {
                            tree_state.expanded.insert(path);
                        }
                    }
                }
                KeyCode::Char('h') | KeyCode::Left => {
                    if let Some(value) = &cell.value {
                        let path = crate::repl::widgets::get_path_at_index(
                            value,
                            tree_state,
                            tree_state.selected,
                        );
                        if let Some(path) = path {
                            tree_state.expanded.remove(&path);
                        }
                    }
                }
                _ => {}
            }
        } else {
            self.mode = ReplMode::Normal;
        }
        Ok(())
    }
}
