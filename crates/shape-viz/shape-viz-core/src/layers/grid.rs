//! Grid layer for chart background grid lines

use crate::data::ChartData;
use crate::error::Result;
use crate::layers::{Layer, LayerStage};
use crate::renderer::RenderContext;
use crate::style::ChartStyle;
use crate::theme::ChartTheme;
use crate::viewport::Viewport;
use glam::Vec2;
use serde_json::Value;

/// Grid layer that renders background grid lines
#[derive(Debug)]
pub struct GridLayer {
    enabled: bool,
    show_major_lines: bool,
    show_minor_lines: bool,
    auto_spacing: bool,
    major_spacing_pixels: f32,
    minor_divisions: u32,
    needs_render: bool,
}

impl GridLayer {
    /// Create a new grid layer with default settings
    pub fn new() -> Self {
        Self {
            enabled: true,
            show_major_lines: true,
            show_minor_lines: false, // Disable minor lines
            auto_spacing: true,
            major_spacing_pixels: 50.0,
            minor_divisions: 5,
            needs_render: true,
        }
    }

    /// Enable or disable major grid lines
    pub fn set_show_major_lines(&mut self, show: bool) {
        if self.show_major_lines != show {
            self.show_major_lines = show;
            self.needs_render = true;
        }
    }

    /// Enable or disable minor grid lines
    pub fn set_show_minor_lines(&mut self, show: bool) {
        if self.show_minor_lines != show {
            self.show_minor_lines = show;
            self.needs_render = true;
        }
    }

    /// Set whether to automatically calculate grid spacing
    pub fn set_auto_spacing(&mut self, auto: bool) {
        if self.auto_spacing != auto {
            self.auto_spacing = auto;
            self.needs_render = true;
        }
    }

    /// Set major grid line spacing in pixels
    pub fn set_major_spacing_pixels(&mut self, spacing: f32) {
        if (self.major_spacing_pixels - spacing).abs() > 0.1 {
            self.major_spacing_pixels = spacing;
            self.needs_render = true;
        }
    }

    /// Render horizontal grid lines
    fn render_horizontal_lines(
        &self,
        context: &mut RenderContext,
        viewport: &Viewport,
        theme: &ChartTheme,
    ) {
        let content_rect = viewport.chart_content_rect();
        let chart_bounds = &viewport.chart_bounds;

        // Use shared utility to calculate price levels - MUST match price axis exactly
        let price_levels = crate::utils::calculate_price_levels(
            chart_bounds.price_min,
            chart_bounds.price_max,
            content_rect.height,
        );

        // Draw horizontal grid lines for each price level
        for &price_level in &price_levels {
            // Convert price to screen coordinates
            let chart_pos = glam::Vec2::new(0.0, price_level as f32);
            let screen_pos = viewport.chart_to_screen(chart_pos);

            if screen_pos.y >= content_rect.y
                && screen_pos.y <= content_rect.y + content_rect.height
            {
                let color = theme.colors.grid_major.with_alpha(0.35);

                context.draw_line(
                    [content_rect.x, screen_pos.y],
                    [content_rect.x + content_rect.width, screen_pos.y],
                    color,
                    1.0,
                );
            }
        }
    }

    /// Render vertical grid lines
    fn render_vertical_lines(
        &self,
        context: &mut RenderContext,
        viewport: &Viewport,
        theme: &ChartTheme,
    ) {
        let content_rect = viewport.chart_content_rect();
        let chart_bounds = &viewport.chart_bounds;

        // Calculate optimal spacing for time grid lines - match the axis spacing
        let time_range_seconds = chart_bounds.time_duration().num_seconds() as f64;
        let target_label_count = (content_rect.width / theme.spacing.grid_spacing_min) as i32;
        let time_step_seconds = time_range_seconds / target_label_count as f64;

        // Find nice round time intervals
        let nice_time_step = self.find_nice_time_interval(time_step_seconds);
        let start_timestamp = ((chart_bounds.time_start.timestamp() as f64 / nice_time_step)
            .floor()
            * nice_time_step) as i64;

        let mut current_timestamp = start_timestamp;

        while current_timestamp <= chart_bounds.time_end.timestamp() {
            // Convert timestamp to screen coordinates
            let chart_pos = Vec2::new(current_timestamp as f32, 0.0);
            let screen_pos = viewport.chart_to_screen(chart_pos);

            if screen_pos.x >= content_rect.x && screen_pos.x <= content_rect.x + content_rect.width
            {
                let color = theme.colors.grid_major.with_alpha(0.35);
                context.draw_line(
                    [screen_pos.x, content_rect.y],
                    [screen_pos.x, content_rect.y + content_rect.height],
                    color,
                    1.0,
                );
            }

            current_timestamp += nice_time_step as i64;
        }
    }

    /// Find a nice time interval in seconds
    fn find_nice_time_interval(&self, seconds: f64) -> f64 {
        // Common time intervals in seconds
        let intervals = [
            1.0,        // 1 second
            5.0,        // 5 seconds
            10.0,       // 10 seconds
            15.0,       // 15 seconds
            30.0,       // 30 seconds
            60.0,       // 1 minute
            300.0,      // 5 minutes
            600.0,      // 10 minutes
            900.0,      // 15 minutes
            1800.0,     // 30 minutes
            3600.0,     // 1 hour
            14400.0,    // 4 hours
            28800.0,    // 8 hours
            43200.0,    // 12 hours
            86400.0,    // 1 day
            604800.0,   // 1 week
            2592000.0,  // 30 days (approximate month)
            7776000.0,  // 90 days (approximate quarter)
            31536000.0, // 1 year
        ];

        // Find the closest interval
        intervals
            .iter()
            .min_by(|&&a, &&b| {
                let diff_a = (a - seconds).abs();
                let diff_b = (b - seconds).abs();
                diff_a
                    .partial_cmp(&diff_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
            .unwrap_or(seconds)
    }
}

impl Default for GridLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Layer for GridLayer {
    fn name(&self) -> &str {
        "Grid"
    }

    fn stage(&self) -> LayerStage {
        LayerStage::ChartUnderlay
    }

    fn update(
        &mut self,
        _data: &ChartData,
        _viewport: &Viewport,
        _theme: &ChartTheme,
        _style: &ChartStyle,
    ) {
        // Grid doesn't depend on data, but we mark as needing render on viewport/theme changes
        self.needs_render = true;
    }

    fn render(
        &self,
        context: &mut RenderContext,
        _render_pass: &mut wgpu::RenderPass,
    ) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let viewport = context.viewport().clone();
        let theme = context.theme().clone();

        // Render both vertical and horizontal lines
        if self.show_major_lines || self.show_minor_lines {
            self.render_vertical_lines(context, &viewport, &theme);
            self.render_horizontal_lines(context, &viewport, &theme);
        }

        Ok(())
    }

    fn needs_render(&self) -> bool {
        self.needs_render
    }

    fn z_order(&self) -> i32 {
        -100 // Render in background
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.needs_render = true;
        }
    }

    fn get_config(&self) -> Value {
        serde_json::json!({
            "show_major_lines": self.show_major_lines,
            "show_minor_lines": self.show_minor_lines,
            "auto_spacing": self.auto_spacing,
            "major_spacing_pixels": self.major_spacing_pixels,
            "minor_divisions": self.minor_divisions
        })
    }

    fn set_config(&mut self, config: Value) -> Result<()> {
        if let Some(show_major) = config.get("show_major_lines").and_then(|v| v.as_bool()) {
            self.set_show_major_lines(show_major);
        }
        if let Some(show_minor) = config.get("show_minor_lines").and_then(|v| v.as_bool()) {
            self.set_show_minor_lines(show_minor);
        }
        if let Some(auto_spacing) = config.get("auto_spacing").and_then(|v| v.as_bool()) {
            self.set_auto_spacing(auto_spacing);
        }
        if let Some(spacing) = config.get("major_spacing_pixels").and_then(|v| v.as_f64()) {
            self.set_major_spacing_pixels(spacing as f32);
        }
        if let Some(divisions) = config.get("minor_divisions").and_then(|v| v.as_u64()) {
            self.minor_divisions = divisions as u32;
            self.needs_render = true;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_layer_creation() {
        let layer = GridLayer::new();
        assert_eq!(layer.name(), "Grid");
        assert!(layer.is_enabled());
        assert!(layer.needs_render());
        assert_eq!(layer.z_order(), -100);
    }

    #[test]
    fn test_grid_layer_configuration() {
        let mut layer = GridLayer::new();

        layer.set_show_major_lines(false);
        assert!(!layer.show_major_lines);
        assert!(layer.needs_render());

        layer.set_auto_spacing(false);
        assert!(!layer.auto_spacing);

        layer.set_major_spacing_pixels(100.0);
        assert_eq!(layer.major_spacing_pixels, 100.0);
    }

    #[test]
    fn test_time_interval_calculation() {
        let layer = GridLayer::new();

        assert_eq!(layer.find_nice_time_interval(3.0), 1.0); // Rounds to 1 second
        assert_eq!(layer.find_nice_time_interval(50.0), 60.0); // Rounds to 1 minute
        assert_eq!(layer.find_nice_time_interval(400.0), 300.0); // Rounds to 5 minutes
        assert_eq!(layer.find_nice_time_interval(3000.0), 3600.0); // Rounds to 1 hour
    }
}
