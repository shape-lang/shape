//! TUI REPL using ratatui
//!
//! Provides an advanced notebook-style REPL with:
//! - Scrolling history of Input/Output cells
//! - Modal editing (Normal, Insert, Command)
//! - Interactive result inspection
//! - Chart visualization

mod cells;
mod events;
mod rendering;
mod state;
pub mod widgets;

pub use cells::{Cell, CellType};
pub use events::ReplMode;
pub use state::ReplState;

use std::io::{self, Stdout};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{
        self, Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use ratatui::prelude::*;

use shape_runtime::engine::ShapeEngine;
use shape_runtime::hashing::HashDigest;
use shape_runtime::snapshot::SnapshotStore;
use shape_wire::{ValueEnvelope, WireValue};

use crate::commands::{ExecutionMode, ProviderOptions};
use crate::extension_loading;
use crate::ui::ProgressDisplay;
use shape_runtime::progress::ProgressEvent;

/// Main TUI REPL application
pub struct ReplApp<'a> {
    /// Current mode
    pub(crate) mode: ReplMode,

    /// Command input (for ':' commands)
    pub(crate) command_input: String,

    /// REPL state (cells, history)
    pub(crate) state: ReplState<'a>,

    /// Shape execution engine
    engine: ShapeEngine,

    /// Scroll offset for the main view
    pub(crate) scroll_offset: usize,

    /// Selected cell index (for inspection)
    pub(crate) selected_cell_index: Option<usize>,

    /// Status message
    pub(crate) status_message: Option<(String, Instant)>,

    /// Pending keys for multi-key commands (like "dd")
    pub(crate) pending_keys: String,

    /// Whether the app should quit
    pub(crate) should_quit: bool,

    /// Extension modules loaded at startup
    _extensions: Vec<PathBuf>,

    /// Progress registry for monitoring load operations
    progress_registry: Option<std::sync::Arc<shape_runtime::progress::ProgressRegistry>>,

    /// Current progress display state
    pub(crate) progress_display: Option<ProgressDisplay>,

    /// Execution mode (VM or JIT)
    execution_mode: ExecutionMode,

    /// Snapshot store (for checkpoint-based resumability)
    snapshot_store: Option<SnapshotStore>,
}

impl<'a> ReplApp<'a> {
    /// Create a new REPL application
    pub fn new(
        mut engine: ShapeEngine,
        extensions: Vec<PathBuf>,
        execution_mode: ExecutionMode,
    ) -> Self {
        // Enable progress tracking on the engine
        let progress_registry = Some(engine.enable_progress_tracking());
        let snapshot_store = engine.snapshot_store().cloned();

        Self {
            mode: ReplMode::Insert,
            command_input: String::new(),
            state: ReplState::new(),
            engine,
            scroll_offset: 0,
            selected_cell_index: None,
            status_message: None,
            pending_keys: String::new(),
            should_quit: false,
            _extensions: extensions,
            progress_registry,
            progress_display: None,
            execution_mode,
            snapshot_store,
        }
    }

    /// Run the TUI REPL
    pub async fn run(&mut self) -> Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let supports_keyboard_enhancement = matches!(supports_keyboard_enhancement(), Ok(true));
        if supports_keyboard_enhancement {
            execute!(
                stdout,
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                )
            )?;
        }
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Main loop
        let result = self.main_loop(&mut terminal).await;

        // Restore terminal
        if supports_keyboard_enhancement {
            let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
        }
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        result
    }

    async fn main_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        loop {
            // Poll for progress events
            self.poll_progress_events();

            terminal.draw(|frame| self.render(frame))?;

            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.handle_key(key).await?;
                    }
                }
            }

            if let Some((_, instant)) = &self.status_message {
                if instant.elapsed() > Duration::from_secs(3) {
                    self.status_message = None;
                }
            }

            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    /// Poll for progress events and update display
    fn poll_progress_events(&mut self) {
        if let Some(registry) = &self.progress_registry {
            while let Some(event) = registry.try_recv() {
                match event {
                    ProgressEvent::Phase { source, phase, .. } => {
                        self.progress_display = Some(ProgressDisplay {
                            source,
                            phase: phase.as_str().to_string(),
                            rows_processed: None,
                            total_rows: None,
                        });
                    }
                    ProgressEvent::Progress {
                        rows_processed,
                        total_rows,
                        ..
                    } => {
                        if let Some(display) = &mut self.progress_display {
                            display.rows_processed = Some(rows_processed);
                            display.total_rows = total_rows;
                        }
                    }
                    ProgressEvent::Complete { .. } | ProgressEvent::Error { .. } => {
                        // Clear progress display when complete
                        self.progress_display = None;
                    }
                }
            }
        }
    }

    pub(crate) async fn execute_focused_input(&mut self, enter_insert_on_next: bool) -> Result<()> {
        if let Some((idx, source)) = self.state.commit_focused_input() {
            self.execute_cell_with_content(idx, source, enter_insert_on_next)
                .await
        } else {
            Ok(())
        }
    }

    pub(crate) async fn execute_cell(
        &mut self,
        index: usize,
        enter_insert_on_next: bool,
    ) -> Result<()> {
        if let Some(source) = self.state.commit_cell(index) {
            self.execute_cell_with_content(index, source, enter_insert_on_next)
                .await
        } else {
            Ok(())
        }
    }

    async fn execute_cell_with_content(
        &mut self,
        idx: usize,
        source: String,
        enter_insert_on_next: bool,
    ) -> Result<()> {
        if source.trim().is_empty() {
            // If it was the *very last* cell, add a new one
            if idx == self.state.cells.len() - 1 {
                self.state.add_active_input();
            }
            return Ok(());
        }

        // Determine if this is a historical cell
        let last_input_idx = self
            .state
            .cells
            .iter()
            .rposition(|c| c.kind == CellType::Input)
            .unwrap_or(idx);
        let is_historical = idx < last_input_idx;
        if is_historical {
            let checkpoint = self
                .state
                .cells
                .iter()
                .take(idx)
                .rev()
                .find_map(|c| c.snapshot_id.clone());

            if let Some(snapshot_id) = checkpoint {
                self.set_status("Restoring checkpoint...");
                self.restore_engine_from_snapshot(&snapshot_id)?;
                self.set_status("Checkpoint restored");
            } else {
                self.set_status("Resetting engine...");
                self.reset_engine()?;
                self.set_status("Engine reset");
            }

            self.state.invalidate_after(idx);
            self.selected_cell_index = Some(idx);
        }

        // Run this single cell (after restoring if needed)
        let start = Instant::now();
        let snapshot_before = self.engine.last_snapshot().cloned();
        let result = self.run_engine(&source).await;
        let elapsed_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(exec_result) => {
                // Suppress Null output
                if matches!(exec_result.value, WireValue::Null) {
                    self.set_status(&format!("Executed in {}ms", elapsed_ms));
                    self.state.remove_output_for(idx);
                } else if let WireValue::PrintResult(ref print_result) = exec_result.value {
                    self.state
                        .update_print_for(idx, print_result.rendered.clone());
                    self.set_status(&format!("Executed in {}ms", elapsed_ms));
                } else {
                    let wire_value = exec_result.value;
                    let envelope = if let Some(type_info) = exec_result.type_info {
                        ValueEnvelope::new(
                            wire_value.clone(),
                            type_info,
                            shape_wire::metadata::TypeRegistry::default_for_primitives(),
                        )
                    } else {
                        ValueEnvelope::from_value(wire_value.clone())
                    };

                    let json_value =
                        serde_json::to_value(&wire_value).unwrap_or(serde_json::Value::Null);

                    self.state
                        .update_output_for(idx, json_value, envelope, &mut self.engine);
                    self.set_status(&format!("Executed in {}ms", elapsed_ms));
                }
                // Mark input cell as executed
                if let Some(cell) = self.state.cells.get_mut(idx) {
                    cell.executed = true;
                }
            }
            Err(e) => {
                self.state.update_error_for(idx, e.to_string());
                self.set_status("Error");
            }
        }

        // Always store a snapshot after execution for checkpoint-based re-execution
        if let Some(cell) = self.state.cells.get_mut(idx) {
            match self.engine.snapshot_with_hashes(None, None) {
                Ok(hash) => cell.snapshot_id = Some(hash),
                Err(_) => {
                    cell.snapshot_id = self.engine.last_snapshot().cloned().or(snapshot_before);
                }
            }
        }

        // If the last cell is not an input (because we added output/error), add a new input
        if let Some(last) = self.state.cells.last() {
            if last.kind != CellType::Input {
                self.state.add_active_input();
            } else if enter_insert_on_next && idx == self.state.cells.len().saturating_sub(1) {
                // Ensure a new input cell even when execution produces no output.
                self.state.add_active_input();
            }
        }

        // Select next input or new input
        let mut entered_insert = false;
        if idx >= last_input_idx {
            if let Some(last_idx) = self.state.cells.len().checked_sub(1) {
                if self.state.cells[last_idx].kind == CellType::Input {
                    self.selected_cell_index = Some(last_idx);
                    // Auto-enter edit mode for new cell?
                    if enter_insert_on_next {
                        self.state.enter_edit_mode(last_idx);
                        self.mode = ReplMode::Insert;
                        entered_insert = true;
                    }
                }
            }
        }

        if !entered_insert {
            self.mode = ReplMode::Normal;
        }
        Ok(())
    }

    fn reset_engine(&mut self) -> Result<()> {
        let mut engine = ShapeEngine::new()?;
        engine.load_stdlib()?;
        engine.init_repl();
        if let Some(store) = &self.snapshot_store {
            engine.enable_snapshot_store(store.clone());
        }

        // Re-enable progress tracking
        self.progress_registry = Some(engine.enable_progress_tracking());

        let provider_opts = ProviderOptions::default();
        let project_root = extension_loading::detect_project_root_for_script(None);
        let startup_specs = extension_loading::collect_startup_specs(
            &provider_opts,
            project_root.as_ref(),
            None,
            None,
            &self._extensions,
        );
        let _ = extension_loading::load_specs(&mut engine, &startup_specs, |_, _| {}, |_, _| {});

        self.engine = engine;
        Ok(())
    }

    async fn rerun_all_cells(&mut self) -> Result<()> {
        self.reset_engine()?;

        // Collect indices first to avoid borrowing issues while iterating
        let input_indices: Vec<usize> = self
            .state
            .cells
            .iter()
            .enumerate()
            .filter(|(_, c)| c.kind == CellType::Input)
            .map(|(i, _)| i)
            .collect();

        for idx in input_indices {
            let source = self.state.cells[idx].content.clone();
            if source.trim().is_empty() {
                continue;
            }

            // Using run_engine on the new engine
            let result = self.run_engine(&source).await;

            match result {
                Ok(exec_result) => {
                    if matches!(exec_result.value, WireValue::Null) {
                        self.state.remove_output_for(idx);
                    } else if let WireValue::PrintResult(ref print_result) = exec_result.value {
                        self.state
                            .update_print_for(idx, print_result.rendered.clone());
                    } else {
                        let wire_value = exec_result.value;
                        let envelope = if let Some(type_info) = exec_result.type_info {
                            ValueEnvelope::new(
                                wire_value.clone(),
                                type_info,
                                shape_wire::metadata::TypeRegistry::default_for_primitives(),
                            )
                        } else {
                            ValueEnvelope::from_value(wire_value.clone())
                        };

                        let json_value =
                            serde_json::to_value(&wire_value).unwrap_or(serde_json::Value::Null);

                        self.state
                            .update_output_for(idx, json_value, envelope, &mut self.engine);
                    }
                }
                Err(e) => {
                    self.state.update_error_for(idx, e.to_string());
                }
            }
        }
        Ok(())
    }

    fn restore_engine_from_snapshot(&mut self, snapshot_id: &HashDigest) -> Result<()> {
        let (semantic, context, _vm_hash, _bytecode_hash) =
            self.engine.load_snapshot(snapshot_id)?;
        self.engine.apply_snapshot(semantic, context)?;
        Ok(())
    }

    async fn run_engine(&mut self, source: &str) -> Result<shape_runtime::engine::ExecutionResult> {
        match self.execution_mode {
            ExecutionMode::BytecodeVM => {
                let mut executor = shape_vm::BytecodeExecutor::new();
                extension_loading::register_extension_capability_modules(
                    &self.engine,
                    &mut executor,
                );
                let module_info = executor.module_schemas();
                self.engine.register_extension_modules(&module_info);
                self.engine.register_language_runtime_artifacts();
                let current_file = std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join("__shape_repl__.shape");
                crate::module_loading::wire_vm_executor_module_loading(
                    &mut self.engine,
                    &mut executor,
                    Some(&current_file),
                    Some(source),
                )?;
                self.engine
                    .execute_repl(&mut executor, source)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))
            }
            #[cfg(feature = "jit")]
            ExecutionMode::JIT => {
                let mut executor = shape_jit::JITExecutor;
                self.engine
                    .execute_repl(&mut executor, source)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))
            }
        }
    }

    pub(crate) async fn execute_command(&mut self, cmd: &str) -> Result<()> {
        match cmd {
            "q" | "quit" => self.should_quit = true,
            "clear" => self.state.clear(),
            _ => self.set_status("Unknown command"),
        }
        Ok(())
    }

    pub(crate) fn set_status(&mut self, msg: &str) {
        self.status_message = Some((msg.to_string(), Instant::now()));
    }
}
