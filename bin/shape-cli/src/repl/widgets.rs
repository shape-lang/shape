use crate::repl::cells::TreeState;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Chart, Dataset, GraphType, StatefulWidget, Widget},
};
use serde_json::Value;

use shape_wire::ValueEnvelope;

pub struct JsonTreeWidget<'a> {
    value: &'a Value,
    envelope: Option<&'a ValueEnvelope>,
}

impl<'a> JsonTreeWidget<'a> {
    pub fn new(value: &'a Value) -> Self {
        Self {
            value,
            envelope: None,
        }
    }

    pub fn with_envelope(mut self, envelope: &'a ValueEnvelope) -> Self {
        self.envelope = Some(envelope);
        self
    }

    pub fn measure_height(value: &Value, state: &TreeState) -> usize {
        let mut count = 0;
        // Add 1 for metadata line if we assume it might be there (approximation)
        // Or we just assume it's small.
        count_lines(value, &mut count, "", state);
        count + 2 // Extra buffer for metadata
    }
}

impl<'a> StatefulWidget for JsonTreeWidget<'a> {
    type State = TreeState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let mut lines = Vec::new();

        // Add metadata line if present
        if let Some(env) = self.envelope {
            let meta_text = format!(
                "Type: {} (Default Format: {})",
                env.type_info.name,
                env.default_format()
            );
            lines.push((
                Line::from(vec![
                    Span::styled(
                        "META ",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(meta_text),
                ]),
                String::new(), // Metadata isn't selectable in tree path logic currently
            ));

            // Add custom metadata if present
            if let Some(meta) = &env.type_info.metadata {
                let meta_str = format!("Metadata: {} entries", meta.sections.len());
                lines.push((
                    Line::from(vec![
                        Span::styled(
                            "META ",
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(meta_str),
                    ]),
                    String::new(),
                ));
            }
        }

        flatten_value(self.value, &mut lines, 0, "", state, None);

        for (i, (line, _path)) in lines.iter().enumerate() {
            if i >= area.height as usize {
                break;
            }

            let y = area.y + i as u16;
            if y >= area.bottom() {
                break;
            }

            // Adjust selection index for metadata lines?
            // If metadata lines are present, they shift the tree.
            // But state.selected refers to the visual index?
            // Or the logical index?
            // If it's visual index, then we just highlight the i-th line.

            let style = if i == state.selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            buf.set_line(area.x, y, line, area.width);
            // Highlight the line if selected
            if i == state.selected {
                buf.set_style(Rect::new(area.x, y, area.width, 1), style);
            }
        }
    }
}

fn flatten_value(
    value: &Value,
    lines: &mut Vec<(Line<'static>, String)>,
    depth: usize,
    path: &str,
    state: &TreeState,
    key: Option<String>,
) {
    let indent = "  ".repeat(depth);
    let key_span = if let Some(k) = key {
        Span::styled(format!("{}: ", k), Style::default().fg(Color::Cyan))
    } else {
        Span::raw("")
    };

    match value {
        Value::Object(map) => {
            let is_expanded = state.expanded.contains(path) || path.is_empty(); // Root expanded?

            let prefix = if is_expanded { "▼ " } else { "▶ " };
            let type_info = format!("{{}} ({} keys)", map.len());

            lines.push((
                Line::from(vec![
                    Span::raw(indent.clone()),
                    key_span,
                    Span::styled(prefix, Style::default().fg(Color::Blue)),
                    Span::raw(type_info),
                ]),
                path.to_string(),
            ));

            if is_expanded {
                for (k, v) in map {
                    let child_path = format!("{}/{}", path, k);
                    flatten_value(v, lines, depth + 1, &child_path, state, Some(k.clone()));
                }
            }
        }
        Value::Array(arr) => {
            let is_expanded = state.expanded.contains(path);

            let prefix = if is_expanded { "▼ " } else { "▶ " };
            let type_info = format!("[] ({} items)", arr.len());

            lines.push((
                Line::from(vec![
                    Span::raw(indent.clone()),
                    key_span,
                    Span::styled(prefix, Style::default().fg(Color::Blue)),
                    Span::raw(type_info),
                ]),
                path.to_string(),
            ));

            if is_expanded {
                for (i, v) in arr.iter().enumerate() {
                    let child_path = format!("{}/{}", path, i);
                    flatten_value(v, lines, depth + 1, &child_path, state, Some(i.to_string()));
                }
            }
        }
        _ => {
            let val_str = value.to_string();
            // Handle long strings?
            lines.push((
                Line::from(vec![
                    Span::raw(indent),
                    key_span,
                    Span::styled(val_str, Style::default().fg(Color::Green)),
                ]),
                path.to_string(),
            ));
        }
    }
}

fn count_lines(value: &Value, count: &mut usize, path: &str, state: &TreeState) {
    *count += 1; // Header/Value line

    match value {
        Value::Object(map) => {
            if state.expanded.contains(path) || path.is_empty() {
                for (k, v) in map {
                    let child_path = format!("{}/{}", path, k);
                    count_lines(v, count, &child_path, state);
                }
            }
        }
        Value::Array(arr) => {
            if state.expanded.contains(path) {
                for (i, v) in arr.iter().enumerate() {
                    let child_path = format!("{}/{}", path, i);
                    count_lines(v, count, &child_path, state);
                }
            }
        }
        _ => {}
    }
}

/// Map a visual line index to its tree path for expand/collapse operations
pub fn get_path_at_index(value: &Value, state: &TreeState, index: usize) -> Option<String> {
    let mut lines: Vec<(Line<'static>, String)> = Vec::new();
    flatten_value(value, &mut lines, 0, "", state, None);
    lines.get(index).map(|(_, path)| path.clone())
}

pub struct ChartWidget {
    data: Vec<(f64, f64)>,
    title: String,
    x_label: String,
    y_label: String,
}

impl ChartWidget {
    pub fn from_series(value: &Value) -> Option<Self> {
        // Simple heuristic for Series<Number>
        if let Some(obj) = value.as_object() {
            if let Some(values) = obj.get("values").and_then(|v| v.as_array()) {
                let data: Vec<(f64, f64)> = values
                    .iter()
                    .enumerate()
                    .filter_map(|(i, v)| v.as_f64().map(|n| (i as f64, n)))
                    .collect();

                if !data.is_empty() {
                    return Some(Self {
                        data,
                        title: "Series".to_string(),
                        x_label: "Time".to_string(),
                        y_label: "Value".to_string(),
                    });
                }
            }
        }
        None
    }
}

impl Widget for ChartWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.data.is_empty() {
            return;
        }

        let (min_y, max_y) = self
            .data
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), (_, y)| {
                (min.min(*y), max.max(*y))
            });

        let datasets = vec![
            Dataset::default()
                .name("data")
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Cyan))
                .data(&self.data),
        ];

        let x_max = self.data.len() as f64;

        let chart = Chart::new(datasets)
            .block(
                ratatui::widgets::Block::default()
                    .borders(ratatui::widgets::Borders::ALL)
                    .title(self.title),
            )
            .x_axis(
                Axis::default()
                    .title(self.x_label)
                    .style(Style::default().fg(Color::Gray))
                    .bounds([0.0, x_max])
                    .labels(vec![
                        Span::styled("0", Style::default().add_modifier(Modifier::BOLD)),
                        Span::styled(
                            format!("{}", x_max),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                    ]),
            )
            .y_axis(
                Axis::default()
                    .title(self.y_label)
                    .style(Style::default().fg(Color::Gray))
                    .bounds([min_y, max_y])
                    .labels(vec![
                        Span::styled(
                            format!("{:.2}", min_y),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{:.2}", max_y),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                    ]),
            );

        chart.render(area, buf);
    }
}
