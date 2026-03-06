//! Status bar widget for the REPL

use ratatui::{prelude::*, widgets::Widget};

use crate::repl::ReplMode;

/// Progress display state
pub struct ProgressDisplay {
    pub source: String,
    pub phase: String,
    pub rows_processed: Option<u64>,
    pub total_rows: Option<u64>,
}

impl ProgressDisplay {
    /// Format progress for display
    pub fn to_string(&self) -> String {
        if let (Some(processed), Some(total)) = (self.rows_processed, self.total_rows) {
            let pct = if total > 0 {
                (processed * 100) / total
            } else {
                0
            };
            format!(
                "Loading {}... {}% ({}/{})",
                self.source, pct, processed, total
            )
        } else if let Some(processed) = self.rows_processed {
            format!("Loading {}... {} rows", self.source, processed)
        } else {
            format!("Loading {}... {}", self.source, self.phase)
        }
    }
}

/// Status bar showing mode, shortcuts, and messages
pub struct StatusBar<'a> {
    mode: ReplMode,
    output_count: usize,
    snapshot_count: usize,
    message: Option<&'a str>,
    progress: Option<&'a ProgressDisplay>,
}

impl<'a> StatusBar<'a> {
    pub fn new(
        mode: ReplMode,
        output_count: usize,
        snapshot_count: usize,
        message: Option<&'a str>,
    ) -> Self {
        Self {
            mode,
            output_count,
            snapshot_count,
            message,
            progress: None,
        }
    }

    pub fn with_progress(mut self, progress: Option<&'a ProgressDisplay>) -> Self {
        self.progress = progress;
        self
    }

    fn mode_style(&self) -> Style {
        match self.mode {
            ReplMode::Normal => Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            ReplMode::Insert => Style::default()
                .bg(Color::Green)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
            ReplMode::Command => Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
            ReplMode::OutputInspect => Style::default()
                .bg(Color::Magenta)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        }
    }

    fn shortcuts(&self) -> Vec<(&'static str, &'static str)> {
        match self.mode {
            ReplMode::Normal => vec![
                ("i", "Insert"),
                (":", "Command"),
                ("j/k", "Navigate"),
                ("x", "Execute"),
                ("r", "Rerun"),
                ("q", "Quit"),
            ],
            ReplMode::Insert => vec![
                ("Esc", "Normal"),
                ("S-Enter", "Execute"),
                ("Enter", "Newline"),
                ("^L", "Clear"),
            ],
            ReplMode::Command => vec![("Esc", "Cancel"), ("Enter", "Execute")],
            ReplMode::OutputInspect => {
                vec![("Esc", "Exit"), ("j/k", "Navigate"), ("h/l", "Collapse")]
            }
        }
    }
}

impl<'a> Widget for StatusBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear the area
        buf.set_style(area, Style::default().bg(Color::DarkGray));

        let mut x = area.x;

        // Mode indicator
        let mode_text = format!(" {} ", self.mode.display_name());
        let mode_width = mode_text.len() as u16;
        if x + mode_width <= area.right() {
            buf.set_string(x, area.y, &mode_text, self.mode_style());
            x += mode_width + 1;
        }

        // Shortcuts
        let shortcuts = self.shortcuts();
        for (key, action) in shortcuts {
            let shortcut_text = format!(" {}:{} ", key, action);
            let width = shortcut_text.len() as u16;
            if x + width <= area.right() - 20 {
                buf.set_string(
                    x,
                    area.y,
                    &shortcut_text,
                    Style::default().fg(Color::White).bg(Color::DarkGray),
                );
                x += width;
            }
        }

        // Progress indicator (if loading)
        if let Some(progress) = &self.progress {
            let progress_text = format!(" {} ", progress.to_string());
            let width = progress_text.len() as u16;
            if x + width <= area.right() - 20 {
                buf.set_string(
                    x,
                    area.y,
                    &progress_text,
                    Style::default()
                        .fg(Color::Yellow)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                );
                x += width;
            }
        }

        // Right side: output count and message
        let right_text = if let Some(msg) = self.message {
            format!(" {} | ${}:{} ", msg, self.output_count, self.snapshot_count)
        } else {
            format!(" ${}:{} ", self.output_count, self.snapshot_count)
        };
        let right_width = right_text.len() as u16;
        let right_x = area.right().saturating_sub(right_width);

        if right_x > x {
            buf.set_string(
                right_x,
                area.y,
                &right_text,
                Style::default().fg(Color::Cyan).bg(Color::DarkGray),
            );
        }
    }
}
