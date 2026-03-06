//! Generic line series overlay for pre-computed indicator values.

use crate::data::ChartData;
use crate::error::Result;
use crate::layers::{Layer, LayerStage};
use crate::renderer::RenderContext;
use crate::style::ChartStyle;
use crate::theme::{ChartTheme, Color};
use crate::viewport::Viewport;
use serde::{Deserialize, Serialize};

/// Configuration for a line series overlay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineSeriesConfig {
    /// Pre-computed values aligned with candle data.
    pub values: Vec<f64>,
    /// Display name/label for the series.
    pub label: String,
    /// Line color in RGBA (0-1 range).
    pub color: [f32; 4],
    /// Stroke width in pixels.
    pub line_width: f32,
    /// Opacity multiplier (0-1).
    pub opacity: f32,
}

impl Default for LineSeriesConfig {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            label: "Indicator".to_string(),
            color: [0.13, 0.59, 0.95, 1.0], // Blue (#2196f3)
            line_width: 1.5,
            opacity: 1.0,
        }
    }
}

/// A line series overlay that renders pre-computed values.
///
/// Unlike EmaLayer which calculates EMA from candle data, this layer
/// takes pre-computed indicator values and renders them directly.
pub struct LineSeriesLayer {
    enabled: bool,
    needs_render: bool,
    config: LineSeriesConfig,
    points: Vec<[f32; 2]>,
}

impl LineSeriesLayer {
    /// Create a new line series layer with the given configuration.
    pub fn new(config: LineSeriesConfig) -> Self {
        Self {
            enabled: true,
            needs_render: true,
            config,
            points: Vec::new(),
        }
    }

    /// Create a simple line series with values, label, and color.
    pub fn with_values(values: Vec<f64>, label: impl Into<String>, color: Color) -> Self {
        Self::new(LineSeriesConfig {
            values,
            label: label.into(),
            color: color.to_array(),
            line_width: 1.5,
            opacity: 1.0,
        })
    }

    /// Update the values at runtime.
    pub fn set_values(&mut self, values: Vec<f64>) {
        self.config.values = values;
        self.needs_render = true;
    }

    /// Update the color at runtime.
    pub fn set_color(&mut self, color: Color) {
        self.config.color = color.to_array();
        self.needs_render = true;
    }

    fn resolve_color(&self, _theme: &ChartTheme) -> Color {
        let c = self.config.color;
        Color::new(c[0], c[1], c[2], c[3] * self.config.opacity)
    }
}

impl Layer for LineSeriesLayer {
    fn name(&self) -> &str {
        &self.config.label
    }

    fn stage(&self) -> LayerStage {
        LayerStage::ChartIndicator
    }

    fn update(
        &mut self,
        data: &ChartData,
        viewport: &Viewport,
        _theme: &ChartTheme,
        _style: &ChartStyle,
    ) {
        self.points.clear();

        if self.config.values.is_empty() {
            self.needs_render = false;
            return;
        }

        let (start_idx, end_idx) = match data.visible_indices() {
            Some((start, end)) => (start, end),
            None => {
                self.needs_render = false;
                return;
            }
        };

        if start_idx >= end_idx {
            self.needs_render = false;
            return;
        }

        let chart_rect = viewport.chart_content_rect();
        let bounds = &viewport.chart_bounds;
        let duration = bounds.time_duration().num_seconds() as f32;
        if duration <= 0.0 {
            self.needs_render = false;
            return;
        }
        let time_scale = chart_rect.width / duration;
        let time_start = bounds.time_start.timestamp() as f32;

        // Map each visible point
        for (idx, i) in (start_idx..end_idx).enumerate() {
            if idx >= self.config.values.len() {
                break;
            }

            let value = self.config.values[idx];
            if value.is_nan() || value.is_infinite() {
                continue;
            }

            let timestamp = data.main_series.get_x(i);
            let delta = timestamp as f32 - time_start;
            let x = chart_rect.x + delta * time_scale;
            let y = viewport.chart_to_screen_y(value as f32);
            self.points.push([x, y]);
        }

        self.needs_render = self.points.len() > 1;
    }

    fn render(
        &self,
        context: &mut RenderContext,
        _render_pass: &mut wgpu::RenderPass,
    ) -> Result<()> {
        if !self.enabled || self.points.len() < 2 {
            return Ok(());
        }

        let theme = context.theme().clone();
        let color = self.resolve_color(&theme);

        for window in self.points.windows(2) {
            let start = window[0];
            let end = window[1];
            context.draw_line(start, end, color, self.config.line_width);
        }

        Ok(())
    }

    fn needs_render(&self) -> bool {
        self.needs_render
    }

    fn z_order(&self) -> i32 {
        51 // Slightly above EMA (50) to layer multiple indicators
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.needs_render = true;
    }
}
