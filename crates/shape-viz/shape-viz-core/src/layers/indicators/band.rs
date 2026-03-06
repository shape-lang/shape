//! Band layer.
//!
//! Renders a filled area between two lines (upper/lower) and an optional middle line.

use crate::data::ChartData;
use crate::error::Result;
use crate::layers::{Layer, LayerStage};
use crate::renderer::{RenderContext, Vertex};
use crate::style::ChartStyle;
use crate::theme::{ChartTheme, Color};
use crate::viewport::Viewport;
use serde::{Deserialize, Serialize};

/// Configuration for Band layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandConfig {
    /// Upper band values
    pub upper: Vec<f64>,
    /// Middle band values (optional)
    pub middle: Option<Vec<f64>>,
    /// Lower band values
    pub lower: Vec<f64>,
    /// Color for the bands in RGBA (0-1 range).
    pub color: [f32; 4],
    /// Line width for bands
    pub line_width: f32,
    /// Fill opacity between bands (0.0 to 1.0)
    pub fill_opacity: f32,
    /// Display label
    pub label: String,
}

impl Default for BandConfig {
    fn default() -> Self {
        Self {
            upper: Vec::new(),
            middle: None,
            lower: Vec::new(),
            color: [0.486, 0.302, 1.0, 1.0], // Purple (#7c4dff)
            line_width: 1.0,
            fill_opacity: 0.1,
            label: "Band".to_string(),
        }
    }
}

/// Band layer.
pub struct BandLayer {
    enabled: bool,
    needs_render: bool,
    config: BandConfig,
    /// Screen-space points for upper band
    upper_points: Vec<[f32; 2]>,
    /// Screen-space points for middle band
    middle_points: Vec<[f32; 2]>,
    /// Screen-space points for lower band
    lower_points: Vec<[f32; 2]>,
}

impl BandLayer {
    /// Create a new Band layer with default configuration.
    pub fn new(config: BandConfig) -> Self {
        Self {
            enabled: true,
            needs_render: true,
            config,
            upper_points: Vec::new(),
            middle_points: Vec::new(),
            lower_points: Vec::new(),
        }
    }

    /// Create from pre-computed values.
    pub fn from_values(upper: Vec<f64>, middle: Option<Vec<f64>>, lower: Vec<f64>) -> Self {
        Self::new(BandConfig {
            upper,
            middle,
            lower,
            ..Default::default()
        })
    }

    /// Set the color.
    pub fn with_color(mut self, color: Color) -> Self {
        self.config.color = color.to_array();
        self
    }

    /// Set fill opacity.
    pub fn with_fill_opacity(mut self, opacity: f32) -> Self {
        self.config.fill_opacity = opacity;
        self
    }

    /// Set the label.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.config.label = label.into();
        self
    }

    /// Update the color at runtime.
    pub fn set_color(&mut self, color: Color) {
        self.config.color = color.to_array();
        self.needs_render = true;
    }

    fn resolve_color(&self, _theme: &ChartTheme) -> Color {
        let c = self.config.color;
        Color::new(c[0], c[1], c[2], c[3])
    }

    fn resolve_fill_color(&self, theme: &ChartTheme) -> Color {
        let base = self.resolve_color(theme);
        base.with_alpha(self.config.fill_opacity)
    }
}

impl Layer for BandLayer {
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
        self.upper_points.clear();
        self.middle_points.clear();
        self.lower_points.clear();

        let upper = &self.config.upper;
        let lower = &self.config.lower;

        if upper.is_empty() || lower.is_empty() {
            self.needs_render = false;
            return;
        }

        let (start_idx, end_idx) = match data.visible_indices() {
            Some((s, e)) => (s, e),
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

        // Map each point
        for (idx, i) in (start_idx..end_idx).enumerate() {
            if idx >= upper.len() || idx >= lower.len() {
                break;
            }

            let timestamp = data.main_series.get_x(i);
            let delta = timestamp as f32 - time_start;
            let x = chart_rect.x + delta * time_scale;

            // Upper band point
            let upper_val = upper[idx];
            if !upper_val.is_nan() && !upper_val.is_infinite() {
                let y = viewport.chart_to_screen_y(upper_val as f32);
                self.upper_points.push([x, y]);
            }

            // Middle band point
            if let Some(middle) = &self.config.middle {
                if idx < middle.len() {
                    let middle_val = middle[idx];
                    if !middle_val.is_nan() && !middle_val.is_infinite() {
                        let y = viewport.chart_to_screen_y(middle_val as f32);
                        self.middle_points.push([x, y]);
                    }
                }
            }

            // Lower band point
            let lower_val = lower[idx];
            if !lower_val.is_nan() && !lower_val.is_infinite() {
                let y = viewport.chart_to_screen_y(lower_val as f32);
                self.lower_points.push([x, y]);
            }
        }

        self.needs_render = self.upper_points.len() > 1
            || self.middle_points.len() > 1
            || self.lower_points.len() > 1;
    }

    fn render(
        &self,
        context: &mut RenderContext,
        _render_pass: &mut wgpu::RenderPass,
    ) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let theme = context.theme().clone();
        let line_color = self.resolve_color(&theme);
        let fill_color = self.resolve_fill_color(&theme);

        // Draw filled area between upper and lower bands
        if self.upper_points.len() >= 2
            && self.lower_points.len() >= 2
            && self.upper_points.len() == self.lower_points.len()
            && self.config.fill_opacity > 0.0
        {
            // Create triangles between upper and lower bands
            for i in 0..self.upper_points.len() - 1 {
                let u1 = self.upper_points[i];
                let u2 = self.upper_points[i + 1];
                let l1 = self.lower_points[i];
                let l2 = self.lower_points[i + 1];

                // First triangle: u1, l1, u2
                context.add_vertices(&[
                    Vertex::new(u1, fill_color),
                    Vertex::new(l1, fill_color),
                    Vertex::new(u2, fill_color),
                ]);

                // Second triangle: l1, l2, u2
                context.add_vertices(&[
                    Vertex::new(l1, fill_color),
                    Vertex::new(l2, fill_color),
                    Vertex::new(u2, fill_color),
                ]);
            }
        }

        // Draw upper band line
        if self.upper_points.len() >= 2 {
            for window in self.upper_points.windows(2) {
                let start = window[0];
                let end = window[1];
                context.draw_line(start, end, line_color, self.config.line_width);
            }
        }

        // Draw middle band line (slightly thicker)
        if self.middle_points.len() >= 2 {
            for window in self.middle_points.windows(2) {
                let start = window[0];
                let end = window[1];
                context.draw_line(start, end, line_color, self.config.line_width * 1.5);
            }
        }

        // Draw lower band line
        if self.lower_points.len() >= 2 {
            for window in self.lower_points.windows(2) {
                let start = window[0];
                let end = window[1];
                context.draw_line(start, end, line_color, self.config.line_width);
            }
        }

        Ok(())
    }

    fn needs_render(&self) -> bool {
        self.needs_render
    }

    fn z_order(&self) -> i32 {
        25 // Below candlesticks but above grid
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.needs_render = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_band_config_default() {
        let config = BandConfig::default();
        assert!(config.upper.is_empty());
        assert!(config.middle.is_none());
        assert!(config.lower.is_empty());
        assert_eq!(config.fill_opacity, 0.1);
        assert_eq!(config.label, "Band");
    }

    #[test]
    fn test_band_layer_creation() {
        let upper = vec![110.0, 112.0, 115.0];
        let middle = vec![100.0, 102.0, 105.0];
        let lower = vec![90.0, 92.0, 95.0];

        let layer = BandLayer::from_values(upper.clone(), Some(middle.clone()), lower.clone());

        assert_eq!(layer.config.upper, upper);
        assert_eq!(layer.config.middle, Some(middle));
        assert_eq!(layer.config.lower, lower);
    }

    #[test]
    fn test_band_layer_name() {
        let layer = BandLayer::new(BandConfig::default());
        assert_eq!(layer.name(), "Band");
    }
}
