use serde_json::Value;
use shape_runtime::hashing::HashDigest;
use shape_wire::ValueEnvelope;
use tui_textarea::TextArea;

/// Type of a REPL cell
#[derive(Debug, Clone, PartialEq)]
pub enum CellType {
    /// Input code
    Input,
    /// Execution result (value)
    Output,
    /// Error message
    Error,
    /// Chart visualization
    Chart,
}

/// A single cell in the notebook interface
#[derive(Debug, Clone)]
pub struct Cell<'a> {
    /// Unique ID
    pub id: usize,
    /// Cell type
    pub kind: CellType,
    /// Content (source code or output text)
    pub content: String,
    /// Structured value (for inspection)
    pub value: Option<Value>,
    /// Wire envelope (for type info and formatting)
    pub envelope: Option<ValueEnvelope>,
    /// Execution time in ms (for inputs)
    pub execution_time: Option<u64>,
    /// Snapshot ID produced by checkpoint() in this cell (if any)
    pub snapshot_id: Option<HashDigest>,
    /// Whether the cell is collapsed
    pub collapsed: bool,
    /// Whether the cell is focused (for inspection)
    pub focused: bool,
    /// Input editor (only for active input cell)
    pub input_editor: Option<TextArea<'a>>,
    /// State for tree view (if output is structured)
    pub tree_state: Option<TreeState>,
    /// Whether this input cell has been executed
    pub executed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct TreeState {
    /// IDs of expanded nodes (path or hash)
    pub expanded: std::collections::HashSet<String>,
    /// Selected row/node index
    pub selected: usize,
}

impl<'a> Cell<'a> {
    pub fn new_input(id: usize, content: String) -> Self {
        Self {
            id,
            kind: CellType::Input,
            content,
            value: None,
            envelope: None,
            execution_time: None,
            snapshot_id: None,
            collapsed: false,
            focused: false,
            input_editor: None,
            tree_state: None,
            executed: false,
        }
    }

    pub fn new_output(
        id: usize,
        value: Value,
        text: String,
        envelope: Option<ValueEnvelope>,
    ) -> Self {
        Self {
            id,
            kind: CellType::Output,
            content: text,
            value: Some(value),
            envelope,
            execution_time: None,
            snapshot_id: None,
            collapsed: false, // Default to expanded so output is visible
            focused: false,
            input_editor: None,
            tree_state: Some(TreeState::default()),
            executed: false,
        }
    }

    pub fn new_error(id: usize, error: String) -> Self {
        Self {
            id,
            kind: CellType::Error,
            content: error,
            value: None,
            envelope: None,
            execution_time: None,
            snapshot_id: None,
            collapsed: false,
            focused: false,
            input_editor: None,
            tree_state: None,
            executed: false,
        }
    }

    pub fn active_input(id: usize) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        textarea.set_placeholder_text("Enter code...");

        Self {
            id,
            kind: CellType::Input,
            content: String::new(),
            value: None,
            envelope: None,
            execution_time: None,
            snapshot_id: None,
            collapsed: false,
            focused: true,
            input_editor: Some(textarea),
            tree_state: None,
            executed: false,
        }
    }
}
