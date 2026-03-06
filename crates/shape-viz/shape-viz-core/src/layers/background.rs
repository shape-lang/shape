//! Background layer for rendering chart outer background using theme colors

use crate::data::ChartData;
use crate::error::Result;
use crate::layers::{Layer, LayerStage};
use crate::renderer::RenderContext;
use crate::style::ChartStyle;
use crate::theme::ChartTheme;
use crate::viewport::Viewport;

/// Layer that fills the entire screen rectangle with the theme's chart_background color.
#[derive(Debug)]
pub struct BackgroundLayer {
    enabled: bool,
    needs_render: bool,
}

impl BackgroundLayer {
    /// Create new background layer
    pub fn new() -> Self {
        Self {
            enabled: true,
            needs_render: true,
        }
    }
}

impl Default for BackgroundLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Layer for BackgroundLayer {
    fn name(&self) -> &str {
        "Background"
    }

    fn stage(&self) -> LayerStage {
        LayerStage::ScreenBackground
    }

    fn update(
        &mut self,
        _data: &ChartData,
        _viewport: &Viewport,
        _theme: &ChartTheme,
        _style: &ChartStyle,
    ) {
        // nothing dynamic, but mark dirty every frame in case size/theme change
        self.needs_render = true;
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
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
        let rect = viewport.screen_rect;
        context.draw_rect(rect, theme.colors.chart_background);
        Ok(())
    }

    fn needs_render(&self) -> bool {
        self.needs_render
    }

    fn z_order(&self) -> i32 {
        -1000 // Furthest back
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }
}
