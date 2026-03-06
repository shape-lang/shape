//! Watermark rendering layer

use crate::data::ChartData;
use crate::error::Result;
use crate::layers::{Layer, LayerStage};
use crate::renderer::RenderContext;
use crate::style::ChartStyle;
use crate::theme::{ChartTheme, Color};
use crate::viewport::Viewport;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Configuration for watermark appearance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatermarkConfig {
    /// The text to display
    pub text: String,
    /// Font size multiplier (relative to viewport)
    pub font_size_factor: f32,
    /// Opacity (0.0 to 1.0)
    pub opacity: f32,
    /// Position (0.0 to 1.0 for x and y)
    pub position: (f32, f32),
    /// Custom color (if None, uses theme)
    pub color: Option<[f32; 4]>,
    /// Bold text
    pub bold: bool,
}

impl Default for WatermarkConfig {
    fn default() -> Self {
        Self {
            text: String::new(),
            font_size_factor: 0.12,            // Smaller text
            opacity: 0.04,                     // Very subtle - almost invisible
            position: (0.5, 0.45),             // Slightly above center
            color: Some([0.4, 0.4, 0.4, 1.0]), // Dark gray color
            bold: true,
        }
    }
}

/// Layer for rendering symbol watermark
#[derive(Debug)]
pub struct WatermarkLayer {
    enabled: bool,
    needs_render: bool,
    config: WatermarkConfig,
}

impl WatermarkLayer {
    pub fn new(symbol: String) -> Self {
        let mut config = WatermarkConfig::default();
        // For demo, add company name below symbol like reference
        config.text = if symbol == "AMZN" || symbol == "DEMO" {
            "AMZN\n\nAmazon.com".to_string()
        } else {
            symbol
        };
        config.font_size_factor = 0.25; // Larger text like reference
        Self {
            enabled: true,
            needs_render: true,
            config,
        }
    }

    pub fn with_config(config: WatermarkConfig) -> Self {
        Self {
            enabled: true,
            needs_render: true,
            config,
        }
    }

    pub fn set_symbol(&mut self, symbol: String) {
        self.config.text = symbol;
        self.needs_render = true;
    }

    pub fn set_config(&mut self, config: WatermarkConfig) {
        self.config = config;
        self.needs_render = true;
    }
}

impl Layer for WatermarkLayer {
    fn name(&self) -> &str {
        "Watermark"
    }

    fn stage(&self) -> LayerStage {
        LayerStage::ChartBackground
    }

    fn update(
        &mut self,
        data: &ChartData,
        _viewport: &Viewport,
        _theme: &ChartTheme,
        _style: &ChartStyle,
    ) {
        // Update watermark text based on actual symbol
        let symbol = data.symbol();
        self.config.text = if symbol == "AMZN" {
            "AMZN\n\nAmazon.com".to_string()
        } else {
            symbol.to_string()
        };
        self.needs_render = true;
    }

    fn render(
        &self,
        context: &mut RenderContext,
        _render_pass: &mut wgpu::RenderPass,
    ) -> Result<()> {
        if !self.enabled || self.config.text.is_empty() {
            return Ok(());
        }

        let viewport = context.viewport().clone();
        let theme = context.theme().clone();
        let content_rect = viewport.chart_content_rect();

        // Calculate font size based on viewport height
        let font_size = content_rect.height * self.config.font_size_factor;

        // Calculate position
        let x = content_rect.x + content_rect.width * self.config.position.0;
        let y = content_rect.y + content_rect.height * self.config.position.1;

        // Get color with opacity
        let base_color = self
            .config
            .color
            .map(|c| Color {
                r: c[0],
                g: c[1],
                b: c[2],
                a: c[3],
            })
            .unwrap_or(theme.colors.text_secondary);

        let color = Color {
            r: base_color.r,
            g: base_color.g,
            b: base_color.b,
            a: base_color.a * self.config.opacity,
        };

        #[cfg(feature = "text-rendering")]
        {
            use crate::text::{TextAnchor, TextBaseline};
            context.draw_text_anchored(
                &self.config.text,
                x,
                y,
                color,
                Some(font_size),
                TextAnchor::Middle,
                TextBaseline::Middle,
            );
        }

        Ok(())
    }

    fn needs_render(&self) -> bool {
        self.needs_render
    }

    fn z_order(&self) -> i32 {
        -200 // Deep background
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
        if let Ok(new_config) = serde_json::from_value::<WatermarkConfig>(config) {
            self.config = new_config;
            self.needs_render = true;
        }
        Ok(())
    }
}
