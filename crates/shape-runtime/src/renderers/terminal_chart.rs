//! Terminal chart rendering using Braille and block characters.
//!
//! Renders ChartSpec data as text-based charts for terminals that
//! don't support Kitty image protocol. Uses:
//! - Braille patterns (U+2800..U+28FF) for line/scatter/area charts
//! - Block characters (U+2581..U+2588) for bar/histogram charts
//! - ASCII for candlestick wicks

use shape_value::content::{ChartSpec, ChartType};
use std::fmt::Write;

/// Default chart dimensions in character cells.
const DEFAULT_WIDTH: usize = 60;
const DEFAULT_HEIGHT: usize = 20;

/// Render a ChartSpec as a text-based terminal chart.
pub fn render_chart_text(spec: &ChartSpec) -> String {
    let width = spec.width.unwrap_or(DEFAULT_WIDTH);
    let height = spec.height.unwrap_or(DEFAULT_HEIGHT);

    match spec.chart_type {
        ChartType::Line | ChartType::Scatter | ChartType::Area => {
            render_braille_chart(spec, width, height)
        }
        ChartType::Bar | ChartType::Histogram => render_bar_chart(spec, width, height),
        ChartType::Candlestick => render_candlestick_chart(spec, width, height),
        _ => render_braille_chart(spec, width, height),
    }
}

// ========== Braille rendering ==========

/// Braille character base (U+2800). Each braille cell is a 2x4 dot grid.
/// Dot numbering (bit positions):
///   0  3
///   1  4
///   2  5
///   6  7
const BRAILLE_BASE: u32 = 0x2800;

/// A 2D grid of braille dots. Each character cell covers 2 columns x 4 rows of dots.
struct BrailleCanvas {
    /// Width in character cells
    char_width: usize,
    /// Height in character cells
    char_height: usize,
    /// Dot grid: char_height*4 rows of char_width*2 columns
    dots: Vec<Vec<bool>>,
}

impl BrailleCanvas {
    fn new(char_width: usize, char_height: usize) -> Self {
        let dot_rows = char_height * 4;
        let dot_cols = char_width * 2;
        Self {
            char_width,
            char_height,
            dots: vec![vec![false; dot_cols]; dot_rows],
        }
    }

    fn dot_width(&self) -> usize {
        self.char_width * 2
    }

    fn dot_height(&self) -> usize {
        self.char_height * 4
    }

    /// Set a dot at (x, y) in dot coordinates. Origin is top-left.
    fn set(&mut self, x: usize, y: usize) {
        if x < self.dot_width() && y < self.dot_height() {
            self.dots[y][x] = true;
        }
    }

    /// Draw a line between two dot coordinates using Bresenham's algorithm.
    fn line(&mut self, x0: usize, y0: usize, x1: usize, y1: usize) {
        let (mut x0, mut y0) = (x0 as isize, y0 as isize);
        let (x1, y1) = (x1 as isize, y1 as isize);
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;

        loop {
            if x0 >= 0 && y0 >= 0 {
                self.set(x0 as usize, y0 as usize);
            }
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    /// Render the canvas to a string of braille characters.
    fn render(&self) -> String {
        let mut out = String::new();
        for cy in 0..self.char_height {
            for cx in 0..self.char_width {
                let mut code: u32 = 0;
                let dx = cx * 2;
                let dy = cy * 4;
                // Map dots to braille bit positions
                if self.dot_at(dx, dy) {
                    code |= 1 << 0;
                }
                if self.dot_at(dx, dy + 1) {
                    code |= 1 << 1;
                }
                if self.dot_at(dx, dy + 2) {
                    code |= 1 << 2;
                }
                if self.dot_at(dx + 1, dy) {
                    code |= 1 << 3;
                }
                if self.dot_at(dx + 1, dy + 1) {
                    code |= 1 << 4;
                }
                if self.dot_at(dx + 1, dy + 2) {
                    code |= 1 << 5;
                }
                if self.dot_at(dx, dy + 3) {
                    code |= 1 << 6;
                }
                if self.dot_at(dx + 1, dy + 3) {
                    code |= 1 << 7;
                }
                if let Some(ch) = char::from_u32(BRAILLE_BASE + code) {
                    out.push(ch);
                }
            }
            out.push('\n');
        }
        out
    }

    fn dot_at(&self, x: usize, y: usize) -> bool {
        if x < self.dot_width() && y < self.dot_height() {
            self.dots[y][x]
        } else {
            false
        }
    }
}

fn render_braille_chart(spec: &ChartSpec, width: usize, height: usize) -> String {
    let x_chan = spec.channel("x");
    let y_channels = spec.channels_by_name("y");
    if y_channels.is_empty() {
        return chart_placeholder(spec);
    }

    // Use y-axis label area: 8 chars for labels + 1 for axis
    let label_width = 8;
    let chart_char_width = width.saturating_sub(label_width + 1);
    let chart_char_height = height.saturating_sub(2); // 1 for title, 1 for x-axis

    if chart_char_width < 4 || chart_char_height < 2 {
        return chart_placeholder(spec);
    }

    let mut canvas = BrailleCanvas::new(chart_char_width, chart_char_height);

    // Compute y range across all y channels
    let (y_min, y_max) = {
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for ch in &y_channels {
            for &v in &ch.values {
                if v.is_finite() {
                    min = min.min(v);
                    max = max.max(v);
                }
            }
        }
        if min == max {
            (min - 1.0, max + 1.0)
        } else {
            (min, max)
        }
    };

    let dot_w = canvas.dot_width();
    let dot_h = canvas.dot_height();

    for ch in &y_channels {
        let n = ch.values.len();
        if n == 0 {
            continue;
        }

        let x_values: Vec<f64> = if let Some(xc) = &x_chan {
            xc.values.clone()
        } else {
            (0..n).map(|i| i as f64).collect()
        };

        let points: Vec<(usize, usize)> = x_values
            .iter()
            .zip(ch.values.iter())
            .filter(|(_, y)| y.is_finite())
            .map(|(x, y)| {
                let x_min = x_values
                    .iter()
                    .copied()
                    .fold(f64::INFINITY, f64::min);
                let x_max = x_values
                    .iter()
                    .copied()
                    .fold(f64::NEG_INFINITY, f64::max);
                let x_range = if (x_max - x_min).abs() < f64::EPSILON {
                    1.0
                } else {
                    x_max - x_min
                };
                let px = ((x - x_min) / x_range * (dot_w - 1) as f64) as usize;
                let py = ((y_max - y) / (y_max - y_min) * (dot_h - 1) as f64) as usize;
                (px.min(dot_w - 1), py.min(dot_h - 1))
            })
            .collect();

        match spec.chart_type {
            ChartType::Scatter => {
                for &(px, py) in &points {
                    canvas.set(px, py);
                }
            }
            _ => {
                // Line: connect consecutive points
                for pair in points.windows(2) {
                    canvas.line(pair[0].0, pair[0].1, pair[1].0, pair[1].1);
                }
            }
        }
    }

    let mut out = String::new();

    // Title
    if let Some(ref title) = spec.title {
        let _ = writeln!(out, "  {}", title);
    }

    // Render with y-axis labels
    let braille_lines: Vec<&str> = canvas.render().lines().map(|l| l).collect();
    // We need to own the rendered string first
    let rendered = canvas.render();
    let braille_lines: Vec<&str> = rendered.lines().collect();

    for (i, line) in braille_lines.iter().enumerate() {
        // Y-axis label at top, middle, bottom
        let label = if i == 0 {
            format!("{:>7.1}", y_max)
        } else if i == braille_lines.len() / 2 {
            format!("{:>7.1}", (y_min + y_max) / 2.0)
        } else if i == braille_lines.len() - 1 {
            format!("{:>7.1}", y_min)
        } else {
            "       ".to_string()
        };
        let _ = writeln!(out, "{} {}", label, line);
    }

    out
}

// ========== Bar chart rendering ==========

/// Block characters from 1/8 to 8/8 height.
const BLOCK_CHARS: [char; 8] = [
    '\u{2581}', // ▁
    '\u{2582}', // ▂
    '\u{2583}', // ▃
    '\u{2584}', // ▄
    '\u{2585}', // ▅
    '\u{2586}', // ▆
    '\u{2587}', // ▇
    '\u{2588}', // █
];

fn render_bar_chart(spec: &ChartSpec, width: usize, height: usize) -> String {
    let y_channels = spec.channels_by_name("y");
    if y_channels.is_empty() {
        return chart_placeholder(spec);
    }

    let values = &y_channels[0].values;
    if values.is_empty() {
        return chart_placeholder(spec);
    }

    let chart_height = height.saturating_sub(3); // title + x labels + spacing
    if chart_height < 2 {
        return chart_placeholder(spec);
    }

    let y_max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let y_min = 0.0_f64; // bars start from 0

    let mut out = String::new();

    // Title
    if let Some(ref title) = spec.title {
        let _ = writeln!(out, "  {}", title);
    }

    // Determine bar width: each value gets some columns
    let bar_count = values.len();
    let available = width.saturating_sub(2);
    let bar_width = (available / bar_count).max(1).min(4);
    let gap = if bar_width > 1 { 1 } else { 0 };

    // Render rows from top to bottom
    for row in 0..chart_height {
        let threshold_top = y_max - (y_max - y_min) * row as f64 / chart_height as f64;
        let threshold_bot = y_max - (y_max - y_min) * (row + 1) as f64 / chart_height as f64;

        let _ = write!(out, " ");
        for (i, &val) in values.iter().enumerate() {
            if i > 0 && gap > 0 {
                out.push(' ');
            }
            for _ in 0..bar_width {
                if val >= threshold_top {
                    out.push(BLOCK_CHARS[7]); // full block
                } else if val > threshold_bot {
                    // Partial block
                    let frac = (val - threshold_bot) / (threshold_top - threshold_bot);
                    let idx = (frac * 7.0) as usize;
                    out.push(BLOCK_CHARS[idx.min(7)]);
                } else {
                    out.push(' ');
                }
            }
        }
        let _ = writeln!(out);
    }

    // X-axis labels from categories
    if let Some(ref cats) = spec.x_categories {
        let _ = write!(out, " ");
        for (i, cat) in cats.iter().enumerate() {
            if i > 0 && gap > 0 {
                out.push(' ');
            }
            let label: String = cat.chars().take(bar_width).collect();
            let _ = write!(out, "{:width$}", label, width = bar_width);
        }
        let _ = writeln!(out);
    }

    out
}

// ========== Candlestick rendering ==========

fn render_candlestick_chart(spec: &ChartSpec, width: usize, height: usize) -> String {
    let open = spec.channel("open");
    let high = spec.channel("high");
    let low = spec.channel("low");
    let close = spec.channel("close");

    let (open, high, low, close) = match (open, high, low, close) {
        (Some(o), Some(h), Some(l), Some(c)) => (o, h, l, c),
        _ => return chart_placeholder(spec),
    };

    let n = open
        .values
        .len()
        .min(high.values.len())
        .min(low.values.len())
        .min(close.values.len());
    if n == 0 {
        return chart_placeholder(spec);
    }

    let chart_height = height.saturating_sub(2);
    if chart_height < 4 {
        return chart_placeholder(spec);
    }

    // Find price range
    let price_min = low
        .values
        .iter()
        .take(n)
        .copied()
        .fold(f64::INFINITY, f64::min);
    let price_max = high
        .values
        .iter()
        .take(n)
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let price_range = if (price_max - price_min).abs() < f64::EPSILON {
        1.0
    } else {
        price_max - price_min
    };

    let available_cols = width.saturating_sub(10); // label space
    let candle_width = (available_cols / n).max(1).min(3);

    let mut out = String::new();
    if let Some(ref title) = spec.title {
        let _ = writeln!(out, "  {}", title);
    }

    // Render rows
    for row in 0..chart_height {
        let row_price_top = price_max - price_range * row as f64 / chart_height as f64;
        let row_price_bot = price_max - price_range * (row + 1) as f64 / chart_height as f64;

        // Y-axis label
        if row == 0 {
            let _ = write!(out, "{:>8.1} ", price_max);
        } else if row == chart_height - 1 {
            let _ = write!(out, "{:>8.1} ", price_min);
        } else {
            let _ = write!(out, "         ");
        }

        for i in 0..n {
            let o = open.values[i];
            let h = high.values[i];
            let l = low.values[i];
            let c = close.values[i];
            let body_top = o.max(c);
            let body_bot = o.min(c);

            for col in 0..candle_width {
                let is_center = col == candle_width / 2;
                // Check if this row intersects the candle
                if row_price_top >= l && row_price_bot <= h {
                    if row_price_bot <= body_top && row_price_top >= body_bot {
                        // Body
                        if c >= o {
                            out.push('█'); // bullish
                        } else {
                            out.push('▒'); // bearish
                        }
                    } else if is_center {
                        out.push('│'); // wick
                    } else {
                        out.push(' ');
                    }
                } else {
                    out.push(' ');
                }
            }
        }
        let _ = writeln!(out);
    }

    out
}

fn chart_placeholder(spec: &ChartSpec) -> String {
    let title = spec.title.as_deref().unwrap_or("untitled");
    let type_name = match spec.chart_type {
        ChartType::Line => "Line",
        ChartType::Bar => "Bar",
        ChartType::Scatter => "Scatter",
        ChartType::Area => "Area",
        ChartType::Candlestick => "Candlestick",
        ChartType::Histogram => "Histogram",
        ChartType::BoxPlot => "BoxPlot",
        ChartType::Heatmap => "Heatmap",
        ChartType::Bubble => "Bubble",
    };
    let y_count = spec.channels_by_name("y").len();
    format!("[{} Chart: {} ({} series)]\n", type_name, title, y_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::content::ChartChannel;

    #[test]
    fn test_braille_canvas_basic() {
        let mut canvas = BrailleCanvas::new(4, 2);
        canvas.set(0, 0);
        canvas.set(1, 0);
        let output = canvas.render();
        assert!(!output.is_empty());
        // Should contain braille characters
        for ch in output.chars() {
            if ch != '\n' {
                assert!(ch as u32 >= BRAILLE_BASE);
            }
        }
    }

    #[test]
    fn test_braille_line_chart() {
        let spec = ChartSpec {
            chart_type: ChartType::Line,
            channels: vec![
                ChartChannel {
                    name: "x".into(),
                    label: "X".into(),
                    values: vec![0.0, 1.0, 2.0, 3.0, 4.0],
                    color: None,
                },
                ChartChannel {
                    name: "y".into(),
                    label: "Y".into(),
                    values: vec![1.0, 4.0, 2.0, 5.0, 3.0],
                    color: None,
                },
            ],
            x_categories: None,
            title: Some("Test Line".into()),
            x_label: None,
            y_label: None,
            width: Some(40),
            height: Some(10),
            echarts_options: None,
            interactive: false,
        };
        let output = render_chart_text(&spec);
        assert!(output.contains("Test Line"));
        // Should contain braille characters
        assert!(output.chars().any(|c| c as u32 >= BRAILLE_BASE && c as u32 <= BRAILLE_BASE + 0xFF));
    }

    #[test]
    fn test_bar_chart() {
        let spec = ChartSpec {
            chart_type: ChartType::Bar,
            channels: vec![ChartChannel {
                name: "y".into(),
                label: "Sales".into(),
                values: vec![10.0, 25.0, 15.0, 30.0],
                color: None,
            }],
            x_categories: Some(vec!["Q1".into(), "Q2".into(), "Q3".into(), "Q4".into()]),
            title: Some("Quarterly Sales".into()),
            x_label: None,
            y_label: None,
            width: Some(30),
            height: Some(10),
            echarts_options: None,
            interactive: false,
        };
        let output = render_chart_text(&spec);
        assert!(output.contains("Quarterly Sales"));
        // Should contain block characters
        assert!(output.chars().any(|c| BLOCK_CHARS.contains(&c)));
    }

    #[test]
    fn test_scatter_chart() {
        let spec = ChartSpec {
            chart_type: ChartType::Scatter,
            channels: vec![
                ChartChannel {
                    name: "x".into(),
                    label: "X".into(),
                    values: vec![1.0, 2.0, 3.0],
                    color: None,
                },
                ChartChannel {
                    name: "y".into(),
                    label: "Y".into(),
                    values: vec![2.0, 4.0, 1.0],
                    color: None,
                },
            ],
            x_categories: None,
            title: Some("Scatter".into()),
            x_label: None,
            y_label: None,
            width: Some(30),
            height: Some(8),
            echarts_options: None,
            interactive: false,
        };
        let output = render_chart_text(&spec);
        assert!(output.contains("Scatter"));
    }

    #[test]
    fn test_empty_chart_fallback() {
        let spec = ChartSpec {
            chart_type: ChartType::Line,
            channels: vec![],
            x_categories: None,
            title: Some("Empty".into()),
            x_label: None,
            y_label: None,
            width: None,
            height: None,
            echarts_options: None,
            interactive: false,
        };
        let output = render_chart_text(&spec);
        assert!(output.contains("[Line Chart: Empty (0 series)]"));
    }

    #[test]
    fn test_candlestick_chart() {
        let spec = ChartSpec {
            chart_type: ChartType::Candlestick,
            channels: vec![
                ChartChannel {
                    name: "open".into(),
                    label: "Open".into(),
                    values: vec![100.0, 105.0, 102.0],
                    color: None,
                },
                ChartChannel {
                    name: "high".into(),
                    label: "High".into(),
                    values: vec![110.0, 112.0, 108.0],
                    color: None,
                },
                ChartChannel {
                    name: "low".into(),
                    label: "Low".into(),
                    values: vec![95.0, 100.0, 98.0],
                    color: None,
                },
                ChartChannel {
                    name: "close".into(),
                    label: "Close".into(),
                    values: vec![105.0, 102.0, 106.0],
                    color: None,
                },
            ],
            x_categories: None,
            title: Some("OHLC".into()),
            x_label: None,
            y_label: None,
            width: Some(30),
            height: Some(12),
            echarts_options: None,
            interactive: false,
        };
        let output = render_chart_text(&spec);
        assert!(output.contains("OHLC"));
    }
}
