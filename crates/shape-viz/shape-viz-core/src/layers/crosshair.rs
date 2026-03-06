//! Crosshair rendering layer

use crate::data::ChartData;
use crate::error::Result;
use crate::layers::{Layer, LayerStage};
use crate::renderer::RenderContext;
use crate::style::ChartStyle;
use crate::theme::ChartTheme;
use crate::viewport::{Rect, Viewport};

/// Layer for rendering crosshair
#[derive(Debug)]
pub struct CrosshairLayer {
    /// Whether the layer is enabled
    enabled: bool,
    /// Whether the layer needs to be re-rendered
    needs_render: bool,
    /// Current cursor position in SCREEN coordinates (pixels).
    /// None means there is no active cursor inside the chart area and
    /// nothing should be drawn.
    position: Option<glam::Vec2>,
}

impl CrosshairLayer {
    pub fn new() -> Self {
        Self {
            enabled: true,
            needs_render: true,
            position: None,
        }
    }

    /// Update the cursor position in **screen space**. Call this from the
    /// event system whenever the mouse moves.
    pub fn set_position(&mut self, x: f32, y: f32) {
        let new_pos = glam::Vec2::new(x, y);
        // Only mark for re-render if the position actually changed.
        if self.position != Some(new_pos) {
            self.position = Some(new_pos);
            self.needs_render = true;
        }
    }

    /// Clear the current position – used when the cursor leaves the chart
    /// area so the crosshair disappears.
    pub fn clear_position(&mut self) {
        if self.position.is_some() {
            self.position = None;
            self.needs_render = true;
        }
    }
}

impl Default for CrosshairLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Layer for CrosshairLayer {
    fn name(&self) -> &str {
        "Crosshair"
    }

    fn stage(&self) -> LayerStage {
        LayerStage::Hud
    }

    fn clip_rect(&self, viewport: &Viewport) -> Rect {
        viewport.chart_content_rect()
    }

    fn update(
        &mut self,
        _data: &ChartData,
        _viewport: &Viewport,
        _theme: &ChartTheme,
        _style: &ChartStyle,
    ) {
        // Update does nothing for now. Rendering happens based on the last
        // recorded cursor position. We only re-render if `needs_render` was
        // flagged by `set_position` / `clear_position`.
    }

    fn render(
        &self,
        context: &mut RenderContext,
        _render_pass: &mut wgpu::RenderPass,
    ) -> Result<()> {
        // Early-out if there is no cursor inside the chart or the layer is disabled
        let Some(pos) = self.position else {
            return Ok(());
        };

        // Retrieve viewport & theme
        let viewport = context.viewport();
        let theme = context.theme();

        // Work inside the chart content rectangle only
        let content_rect = viewport.chart_content_rect();

        // Bail out if the cursor is outside the content area
        if pos.x < content_rect.x
            || pos.x > content_rect.x + content_rect.width
            || pos.y < content_rect.y
            || pos.y > content_rect.y + content_rect.height
        {
            return Ok(());
        }

        let color = theme.colors.crosshair;
        let thickness = theme.spacing.crosshair_width;
        // Dash parameters chosen to visually resemble TradingView style
        let dash = 4.0;
        let gap = 4.0;

        // Horizontal line
        context.draw_dashed_line(
            [content_rect.x, pos.y],
            [content_rect.x + content_rect.width, pos.y],
            color,
            thickness,
            dash,
            gap,
        );

        // Vertical line
        context.draw_dashed_line(
            [pos.x, content_rect.y],
            [pos.x, content_rect.y + content_rect.height],
            color,
            thickness,
            dash,
            gap,
        );

        Ok(())
    }

    fn needs_render(&self) -> bool {
        self.needs_render
    }

    fn z_order(&self) -> i32 {
        100 // Top layer
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.needs_render = true;
    }
}
