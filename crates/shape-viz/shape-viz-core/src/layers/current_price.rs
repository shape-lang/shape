//! Current price line layer

use crate::data::ChartData;
use crate::error::Result;
use crate::layers::{Layer, LayerStage};
use crate::renderer::RenderContext;
use crate::style::ChartStyle;
use crate::theme::{ChartTheme, Color};
use crate::viewport::{Rect, Viewport};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Configuration for current price line
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentPriceConfig {
    /// Show the horizontal price line
    pub show_line: bool,
    /// Show the price label box
    pub show_label: bool,
    /// Line style (solid, dashed, dotted)
    pub line_style: LineStyle,
    /// Line width
    pub line_width: f32,
    /// Label padding
    pub label_padding: f32,
    /// Custom line color (if None, uses theme)
    pub line_color: Option<[f32; 4]>,
    /// Custom label background color (if None, uses candle color)
    pub label_bg_color: Option<[f32; 4]>,
    /// Custom label text color (if None, uses theme)
    pub label_text_color: Option<[f32; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LineStyle {
    Solid,
    Dashed,
    Dotted,
}

impl Default for CurrentPriceConfig {
    fn default() -> Self {
        Self {
            show_line: true,
            show_label: true,
            line_style: LineStyle::Dashed, // Dashed line like TradingView
            line_width: 1.0,
            label_padding: 4.0,
            line_color: None,
            label_bg_color: None,
            label_text_color: None,
        }
    }
}

/// Layer for rendering current price line and label
#[derive(Debug)]
pub struct CurrentPriceLayer {
    enabled: bool,
    needs_render: bool,
    config: CurrentPriceConfig,
    current_price: Option<f64>,
    is_bullish: bool,
    symbol: String,
}

impl CurrentPriceLayer {
    pub fn new() -> Self {
        Self {
            enabled: true,
            needs_render: true,
            config: CurrentPriceConfig::default(),
            current_price: None,
            is_bullish: true,
            symbol: String::new(),
        }
    }

    pub fn with_config(config: CurrentPriceConfig) -> Self {
        Self {
            config,
            ..Self::new()
        }
    }

    /// Update configuration
    pub fn set_config(&mut self, config: CurrentPriceConfig) {
        self.config = config;
        self.needs_render = true;
    }

    /// Draw dashed or dotted line
    fn draw_styled_line(
        &self,
        context: &mut RenderContext,
        start: [f32; 2],
        end: [f32; 2],
        color: Color,
        width: f32,
        dash: f32,
        gap: f32,
    ) {
        match self.config.line_style {
            LineStyle::Solid => {
                context.draw_line(start, end, color, width);
            }
            LineStyle::Dashed => {
                let total_length =
                    ((end[0] - start[0]).powi(2) + (end[1] - start[1]).powi(2)).sqrt();
                let dx = (end[0] - start[0]) / total_length;
                let dy = (end[1] - start[1]) / total_length;

                let mut current_length = 0.0;
                let mut drawing = true;

                while current_length < total_length {
                    let segment_length = if drawing { dash } else { gap };
                    let next_length = (current_length + segment_length).min(total_length);

                    if drawing {
                        let x1 = start[0] + dx * current_length;
                        let y1 = start[1] + dy * current_length;
                        let x2 = start[0] + dx * next_length;
                        let y2 = start[1] + dy * next_length;
                        context.draw_line([x1, y1], [x2, y2], color, width);
                    }

                    current_length = next_length;
                    drawing = !drawing;
                }
            }
            LineStyle::Dotted => {
                let dot_spacing = 5.0;
                let total_length =
                    ((end[0] - start[0]).powi(2) + (end[1] - start[1]).powi(2)).sqrt();
                let num_dots = (total_length / dot_spacing) as i32;

                for i in 0..=num_dots {
                    let t = i as f32 / num_dots as f32;
                    let x = start[0] + (end[0] - start[0]) * t;
                    let y = start[1] + (end[1] - start[1]) * t;

                    // Draw small dot
                    context.draw_rect(
                        crate::viewport::Rect::new(x - width / 2.0, y - width / 2.0, width, width),
                        color,
                    );
                }
            }
        }
    }
}

impl Default for CurrentPriceLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Layer for CurrentPriceLayer {
    fn name(&self) -> &str {
        "CurrentPrice"
    }

    fn stage(&self) -> LayerStage {
        LayerStage::Hud // Render after price axis so label appears on top
    }

    fn clip_rect(&self, viewport: &Viewport) -> Rect {
        // Use full screen rect so label renders over price axis without clipping
        viewport.screen_rect
    }

    fn update(
        &mut self,
        data: &ChartData,
        _viewport: &Viewport,
        _theme: &ChartTheme,
        style: &ChartStyle,
    ) {
        self.config.line_width = style.current_price.line_width;
        self.config.label_padding = style.current_price.label_padding;
        // Store name
        self.symbol = data.symbol().to_string();

        // Get the last value (e.g., close price)
        if !data.main_series.is_empty() {
            let last_idx = data.main_series.len() - 1;
            let current_value = data.main_series.get_y(last_idx);
            self.current_price = Some(current_value);

            // Determine if positive or negative (for coloring)
            // Compare current value with previous value
            if last_idx > 0 {
                let prev = data.main_series.get_y(last_idx - 1);
                self.is_bullish = current_value >= prev;
            } else {
                self.is_bullish = true;
            }
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
        if !self.enabled || self.current_price.is_none() {
            return Ok(());
        }

        let price = self.current_price.unwrap();
        let viewport = context.viewport().clone();
        let theme = context.theme().clone();
        let content_rect = viewport.chart_content_rect();
        let price_axis_rect = viewport.price_axis_rect();

        // Convert price to screen Y coordinate
        let screen_y = viewport.chart_to_screen_y(price as f32);

        // Only render if price is visible
        if screen_y < content_rect.y || screen_y > content_rect.y + content_rect.height {
            return Ok(());
        }

        // TradingView style: use candle color for current price
        let candle_color = if self.is_bullish {
            theme.colors.candle_bullish
        } else {
            theme.colors.candle_bearish
        };

        let line_color = self
            .config
            .line_color
            .map(|c| Color {
                r: c[0],
                g: c[1],
                b: c[2],
                a: c[3],
            })
            .unwrap_or(candle_color.with_alpha(0.6));

        let label_bg_color = self
            .config
            .label_bg_color
            .map(|c| Color {
                r: c[0],
                g: c[1],
                b: c[2],
                a: c[3],
            })
            .unwrap_or(candle_color);

        let label_text_color = self
            .config
            .label_text_color
            .map(|c| Color {
                r: c[0],
                g: c[1],
                b: c[2],
                a: c[3],
            })
            .unwrap_or(Color::hex(0xffffff)); // White text on colored background

        // Draw horizontal line across content area
        if self.config.show_line {
            let line_end_x = content_rect.x + content_rect.width;
            let cp_style = &context.style().current_price;
            self.draw_styled_line(
                context,
                [content_rect.x, screen_y],
                [line_end_x, screen_y],
                line_color,
                self.config.line_width,
                cp_style.dash_length,
                cp_style.dash_gap,
            );
        }

        // Draw price label - TradingView style: rounded rect overlapping axis values
        if self.config.show_label {
            let price_text = format!("{:.2}", price);

            // Calculate label size based on text
            let font_size = theme.typography.secondary_font_size;
            let char_width = font_size * 0.6;
            let label_width =
                price_text.len() as f32 * char_width + self.config.label_padding * 2.0;
            let label_height = font_size + self.config.label_padding * 2.0;
            let corner_radius = 3.0;

            // Position: right-aligned within price axis with small margin
            let margin = 4.0;
            let label_x = price_axis_rect.x + price_axis_rect.width - label_width - margin;

            // Center vertically on the price line, clamped to axis bounds
            let mut label_y = screen_y - label_height / 2.0;
            let axis_top = price_axis_rect.y + margin;
            let axis_bottom = price_axis_rect.y + price_axis_rect.height - label_height - margin;
            label_y = label_y.clamp(axis_top, axis_bottom);

            // Draw rounded rectangle background
            let label_rect = Rect::new(label_x, label_y, label_width, label_height);
            context.draw_rounded_rect(label_rect, corner_radius, label_bg_color);

            // Draw left-pointing triangle arrow
            let arrow_width = 6.0;
            let arrow_x = label_x;
            let arrow_y = label_y + label_height / 2.0;

            context.draw_triangle(
                [arrow_x - arrow_width, arrow_y],
                [arrow_x, arrow_y - label_height / 2.0],
                [arrow_x, arrow_y + label_height / 2.0],
                label_bg_color,
            );

            // Draw price text centered in label
            #[cfg(feature = "text-rendering")]
            {
                use crate::text::{TextAnchor, TextBaseline};
                context.draw_text_anchored(
                    &price_text,
                    label_x + label_width / 2.0,
                    label_y + label_height / 2.0,
                    label_text_color,
                    Some(font_size),
                    TextAnchor::Middle,
                    TextBaseline::Middle,
                );
            }
        }

        Ok(())
    }

    fn needs_render(&self) -> bool {
        self.needs_render
    }

    fn z_order(&self) -> i32 {
        70 // Above axes
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
        if let Ok(new_config) = serde_json::from_value::<CurrentPriceConfig>(config) {
            self.config = new_config;
            self.needs_render = true;
        }
        Ok(())
    }
}
