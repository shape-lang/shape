//! Rendering logic for the REPL
//!
//! This module handles all UI rendering, including cells, status bar, and overlays.

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::repl::widgets::{ChartWidget, JsonTreeWidget};
use crate::repl::{Cell, CellType, ReplApp, ReplMode};
use crate::ui::StatusBar;

impl<'a> ReplApp<'a> {
    pub(super) fn render(&mut self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),    // Main scrolling area (Cells)
                Constraint::Length(1), // Status bar
            ])
            .split(frame.area());

        self.render_cells(frame, chunks[0]);
        self.render_status(frame, chunks[1]);

        // Render command input overlay if in Command mode
        if self.mode == ReplMode::Command {
            let area = centered_fixed_height_rect(60, 3, frame.area());
            let block = Block::default()
                .borders(Borders::ALL)
                .title(" Command ")
                .border_style(Style::default().fg(Color::Yellow));
            let text = Paragraph::new(format!(":{}", self.command_input))
                .block(block)
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(Clear, area); // Clear underlying content
            frame.render_widget(text, area);
        }
    }

    fn render_cells(&mut self, frame: &mut Frame, area: Rect) {
        // Measure all cells
        let cell_heights: Vec<usize> = self
            .state
            .cells
            .iter()
            .map(|c| self.measure_cell_height(c, area.width))
            .collect();

        let total_height: usize = cell_heights.iter().sum();
        let viewport_height = area.height as usize;

        // Determine target cell to ensure is visible
        let target_idx = self.selected_cell_index.unwrap_or_else(|| {
            // Default to last cell (active input) if nothing selected
            self.state.cells.len().saturating_sub(1)
        });

        // Calculate Y position range of target cell
        let mut target_top = 0;
        for i in 0..target_idx {
            target_top += cell_heights[i];
        }
        let target_bottom = target_top + cell_heights.get(target_idx).copied().unwrap_or(0);

        // Update scroll_offset to keep target in view
        // 1. Ensure top is visible (if larger than viewport, align top)
        // 2. Ensure bottom is visible (if smaller than viewport)

        // This logic mimics "scroll into view"
        // Since we can't persist scroll_offset easily without refactoring (we are in render),
        // we use a heuristic based on selection.
        // If we wanted smooth scrolling or manual scrolling, we'd need to update self.scroll_offset in handle_key.
        // For now, "snap to selection" is good.

        let scroll_base = if target_bottom > viewport_height + self.scroll_offset {
            // Scroll down to show bottom
            target_bottom.saturating_sub(viewport_height)
        } else if target_top < self.scroll_offset {
            // Scroll up to show top
            target_top
        } else {
            // Keep current scroll if visible
            // But we are recalculating every frame. We need persistence.
            // We can use self.scroll_offset as persistence!
            // But we need to update it.
            // Since we are in render(&mut self), we CAN update self.scroll_offset.
            self.scroll_offset
        };

        // Clamp scroll_base
        let max_scroll = total_height.saturating_sub(viewport_height);
        let scroll_base = scroll_base.min(max_scroll);

        // Update persistent state (hacky but works since we have &mut self)
        self.scroll_offset = scroll_base;

        // Render loop
        let mut current_y = 0; // Absolute Y position in document

        for i in 0..self.state.cells.len() {
            let height = cell_heights[i];
            let cell_top = current_y;
            let cell_bottom = current_y + height;

            // Check visibility
            // Visible if [cell_top, cell_bottom] overlaps [scroll_base, scroll_base + viewport_height]
            if cell_bottom > scroll_base && cell_top < scroll_base + viewport_height {
                // Determine screen Y
                let screen_y_abs = cell_top.saturating_sub(scroll_base);
                let screen_y = screen_y_abs as u16;

                // Handle clipping at top
                let clip_top = scroll_base.saturating_sub(cell_top);

                let visible_height = height.saturating_sub(clip_top);
                // Also clip at bottom if needed
                let remaining_screen = viewport_height.saturating_sub(screen_y as usize);
                let render_height = visible_height.min(remaining_screen);

                let render_area =
                    Rect::new(area.x, area.y + screen_y, area.width, render_height as u16);

                if render_area.height > 0 {
                    let is_selected = self.selected_cell_index == Some(i);
                    render_cell(frame, &mut self.state.cells[i], render_area, is_selected);
                }
            }

            current_y += height;
        }
    }

    fn measure_cell_height(&self, cell: &Cell, _width: u16) -> usize {
        match cell.kind {
            CellType::Input => {
                if let Some(editor) = &cell.input_editor {
                    editor.lines().len().max(1) + 1 // +1 for prompt line
                } else {
                    cell.content.lines().count().max(1)
                }
            }
            CellType::Output => {
                if cell.collapsed {
                    1 // Summary line
                } else {
                    if let Some(value) = &cell.value {
                        if ChartWidget::from_series(value).is_some() {
                            return 15; // Fixed height for charts
                        }
                        if let Some(tree_state) = &cell.tree_state {
                            return JsonTreeWidget::measure_height(value, tree_state).max(1);
                        }
                    }
                    cell.content.lines().count().max(1)
                }
            }
            CellType::Error => {
                cell.content.lines().count().max(1) + 1 // +1 for "Error:" header
            }
            CellType::Chart => 10, // Fixed height for chart placeholder
        }
    }

    pub(super) fn render_status(&self, frame: &mut Frame, area: Rect) {
        let status = StatusBar::new(
            self.mode,
            self.state.cells.len(),
            0,
            self.status_message.as_ref().map(|(msg, _)| msg.as_str()),
        )
        .with_progress(self.progress_display.as_ref());
        frame.render_widget(status, area);
    }
}

fn render_cell(frame: &mut Frame, cell: &mut Cell, area: Rect, is_selected: bool) {
    let gutter_width = 6;
    let content_width = area.width.saturating_sub(gutter_width);

    let gutter_area = Rect::new(area.x, area.y, gutter_width, 1); // ID only on first line
    let content_area = Rect::new(area.x + gutter_width, area.y, content_width, area.height);

    // Highlight selection
    let gutter_style = if is_selected {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    // Render gutter content (ID / Status)
    let (gutter_sym, gutter_color) = match cell.kind {
        CellType::Input => {
            if cell.executed {
                (format!("[{}]✓", cell.id), Color::Cyan)
            } else {
                (format!("[{}]", cell.id), Color::Green)
            }
        }
        CellType::Output => (
            if cell.collapsed { " ▶" } else { " " }.to_string(),
            Color::DarkGray,
        ),
        CellType::Error => (" !".to_string(), Color::Red),
        CellType::Chart => (" #".to_string(), Color::Cyan),
    };

    let final_gutter_style = gutter_style.fg(if is_selected {
        Color::Yellow
    } else {
        gutter_color
    });

    frame.render_widget(
        Paragraph::new(gutter_sym).style(final_gutter_style),
        gutter_area,
    );

    // Draw continuous vertical line in gutter
    let buf = frame.buffer_mut();
    let line_style = Style::default().fg(Color::DarkGray);
    for y in area.y..area.bottom() {
        buf.set_string(area.x + 5, y, "│", line_style);
    }

    // Highlight content background if selected
    if is_selected {
        // Render a block or background for the whole cell area?
        // Or just content area.
        // Let's keep it subtle - just gutter highlight is often enough,
        // but user said "cannot navigate lines up", implying they couldn't SEE the selection.
        // Let's add a left border to content area for selected cell.
        let block = Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(Color::Yellow));
        frame.render_widget(block, content_area);
    }

    let content_widget_area = if is_selected {
        // Adjust for border
        Rect::new(
            content_area.x + 1,
            content_area.y,
            content_width.saturating_sub(1),
            content_area.height,
        )
    } else {
        content_area
    };

    match cell.kind {
        CellType::Input => {
            if let Some(editor) = &cell.input_editor {
                frame.render_widget(editor, content_widget_area);
            } else {
                frame.render_widget(Paragraph::new(cell.content.clone()), content_widget_area);
            }
        }
        CellType::Output => {
            if cell.collapsed {
                // Render summary using content
                let summary = cell.content.lines().next().unwrap_or("").to_string();
                let summary = if summary.len() > (content_width as usize - 5) {
                    format!("{}...", &summary[..content_width as usize - 5])
                } else {
                    summary
                };
                frame.render_widget(
                    Paragraph::new(summary).style(Style::default().fg(Color::DarkGray)),
                    content_widget_area,
                );
            } else {
                // Render full content
                if let Some(value) = &cell.value {
                    if let Some(chart) = ChartWidget::from_series(value) {
                        frame.render_widget(chart, content_widget_area);
                        return;
                    }
                }

                if let Some(tree_state) = &mut cell.tree_state {
                    if let Some(value) = &cell.value {
                        let mut widget = JsonTreeWidget::new(value);
                        if let Some(env) = &cell.envelope {
                            widget = widget.with_envelope(env);
                        }
                        frame.render_stateful_widget(widget, content_widget_area, tree_state);
                        return;
                    }
                }

                let content = Paragraph::new(cell.content.clone());
                frame.render_widget(content, content_widget_area);
            }
        }
        CellType::Error => {
            let content =
                Paragraph::new(cell.content.clone()).style(Style::default().fg(Color::Red));
            frame.render_widget(content, content_widget_area);
        }
        CellType::Chart => {
            frame.render_widget(Paragraph::new("Chart"), content_widget_area);
        }
    }
}

// Helper to center a rect with fixed height (for popups like command input)
pub(super) fn centered_fixed_height_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
