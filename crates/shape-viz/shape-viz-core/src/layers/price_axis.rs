//! Price axis rendering layer

use crate::data::ChartData;
use crate::error::Result;
use crate::layers::{Layer, LayerStage};
use crate::renderer::RenderContext;
use crate::style::ChartStyle;
use crate::theme::ChartTheme;
use crate::viewport::Viewport;
use serde::{Deserialize, Serialize};
use serde_json::Value;
// For Rect::new for debug_rect
// For Color::hex for debug_rect

/// Layer for rendering price axis
/// Configuration for price axis behaviour and appearance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceAxisConfig {
    /// Width (in screen px) of the dedicated price axis column
    pub axis_width: f32,
    /// Should the layer also draw horizontal grid lines that span the chart?
    pub show_grid_lines: bool,
    /// Tick length in px – kept for completeness even though not shown by default
    pub tick_length: f32,
    /// Offset between tick and text label
    pub label_offset: f32,
}

impl Default for PriceAxisConfig {
    fn default() -> Self {
        Self {
            axis_width: 50.0,       // Reduced from 60.0
            show_grid_lines: false, // Disable - grid layer handles this
            tick_length: 5.0,
            label_offset: 4.0, // Reduced from 8.0
        }
    }
}

#[derive(Debug)]
pub struct PriceAxisLayer {
    enabled: bool,
    needs_render: bool,
    config: PriceAxisConfig,
    /// Current price to skip in axis labels (will be drawn by CurrentPriceLayer)
    current_price: Option<f64>,
}

impl PriceAxisLayer {
    pub fn new() -> Self {
        Self {
            enabled: true,
            needs_render: true,
            config: PriceAxisConfig::default(),
            current_price: None,
        }
    }

    /// Set the width of the price axis
    pub fn set_axis_width(&mut self, width: f32) {
        if (self.config.axis_width - width).abs() > 0.1 {
            self.config.axis_width = width;
            self.needs_render = true;
        }
    }

    /// Enable / disable drawing of accompanying horizontal grid lines that align with labels
    pub fn set_show_grid_lines(&mut self, show: bool) {
        if self.config.show_grid_lines != show {
            self.config.show_grid_lines = show;
            self.needs_render = true;
        }
    }

    /// Replace the whole configuration object
    pub fn set_config(&mut self, config: PriceAxisConfig) {
        self.config = config;
        self.needs_render = true;
    }

    /// Format price value for display - always 2 decimals like professional charts
    fn format_price(&self, price: f64) -> String {
        format!("{:.2}", price)
    }
}

impl Default for PriceAxisLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Layer for PriceAxisLayer {
    fn name(&self) -> &str {
        "PriceAxis"
    }

    fn stage(&self) -> LayerStage {
        LayerStage::PriceAxis
    }

    fn update(
        &mut self,
        data: &ChartData,
        _viewport: &Viewport,
        _theme: &ChartTheme,
        style: &ChartStyle,
    ) {
        self.config.axis_width = style.layout.price_axis_width;

        // Store current price (last candle's close) to skip in axis labels
        if !data.main_series.is_empty() {
            let last_idx = data.main_series.len() - 1;
            self.current_price = Some(data.main_series.get_y(last_idx));
        } else {
            self.current_price = None;
        }

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
        let content_rect = viewport.chart_content_rect();
        let axis_rect = viewport.price_axis_rect();
        // Ensure axis_rect width matches current configuration
        if (axis_rect.width - self.config.axis_width).abs() > 0.1 {
            // The viewport implementation owns the rect; we assume it honours config elsewhere.
            // For now we keep rendering logic, but callers should update viewport before render.
        }
        let chart_bounds = &viewport.chart_bounds;

        // The axis line is at the left edge of the axis area (boundary between content and axis)
        let _axis_line_x = content_rect.x + content_rect.width;

        // Draw axis background using dedicated axis background color
        context.draw_rect(axis_rect, theme.colors.axis_background);

        // Don't draw axis line - reference chart has no visible axis lines

        // Use shared utility to calculate price levels
        let price_levels = crate::utils::calculate_price_levels(
            chart_bounds.price_min,
            chart_bounds.price_max,
            content_rect.height,
        );

        // Current price label height (must match CurrentPriceLayer)
        let current_price_label_height = theme.typography.secondary_font_size + 8.0; // font + padding*2
        let pixels_per_price =
            content_rect.height / (chart_bounds.price_max - chart_bounds.price_min) as f32;

        // Draw price labels for each level
        for &price_level in &price_levels {
            // Skip labels that would overlap with current price label
            if let Some(current) = self.current_price {
                let pixel_distance = ((price_level - current) as f32 * pixels_per_price).abs();
                // Only skip if this label's center falls within the current price label's box
                if pixel_distance < current_price_label_height / 2.0 {
                    continue;
                }
            }

            // Convert price to screen coordinates
            let chart_pos = glam::Vec2::new(0.0, price_level as f32);
            let screen_pos = viewport.chart_to_screen(chart_pos);

            if screen_pos.y >= content_rect.y
                && screen_pos.y <= content_rect.y + content_rect.height
            {
                // Don't draw tick marks - reference chart has no visible ticks

                // Draw price label in the axis area
                let price_text = self.format_price(price_level);

                #[cfg(feature = "text-rendering")]
                {
                    use crate::text::{TextAnchor, TextBaseline};
                    context.draw_text_anchored(
                        &price_text,
                        axis_rect.x + axis_rect.width - 5.0, // Right-align labels
                        screen_pos.y,
                        theme.colors.axis_label,
                        Some(theme.typography.secondary_font_size),
                        TextAnchor::End, // Right-align
                        TextBaseline::Middle,
                    );
                }

                // Optionally draw grid line
                if self.config.show_grid_lines {
                    context.draw_line(
                        [content_rect.x, screen_pos.y],
                        [content_rect.x + content_rect.width, screen_pos.y],
                        theme.colors.grid_minor,
                        0.5,
                    );
                }
            }
        }

        Ok(())
    }

    fn needs_render(&self) -> bool {
        self.needs_render
    }

    fn z_order(&self) -> i32 {
        60 // Above grid and candles
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.needs_render = true;
    }

    fn get_config(&self) -> Value {
        serde_json::to_value(&self.config).unwrap_or(Value::Null)
    }

    fn set_config(&mut self, config: Value) -> Result<()> {
        if let Ok(cfg) = serde_json::from_value::<PriceAxisConfig>(config) {
            self.config = cfg;
            self.needs_render = true;
        }
        Ok(())
    }
}
