//! Modular layer system for chart rendering
//!
//! The layer system allows composing complex charts from simple, reusable components.
//! Each layer is responsible for rendering a specific aspect of the chart (candlesticks,
//! grid, axes, indicators, etc.) and can be independently configured and updated.

pub mod auxiliary_bar;
pub mod background;
pub mod crosshair;
pub mod current_price;
pub mod grid;
pub mod indicators;
pub mod price_axis;
pub mod range_bar;
pub mod region_shading;
pub mod time_axis;
pub mod watermark;

// Re-export layer implementations
pub use auxiliary_bar::AuxiliaryBarLayer;
pub use background::BackgroundLayer;
pub use crosshair::CrosshairLayer;
pub use current_price::{CurrentPriceConfig, CurrentPriceLayer, LineStyle};
pub use grid::GridLayer;
pub use indicators::band::{BandConfig, BandLayer};
pub use indicators::line_series::{LineSeriesConfig, LineSeriesLayer};
pub use price_axis::PriceAxisLayer;
pub use range_bar::{RangeBarConfig, RangeBarLayer, RangeBarStyle};
pub use region_shading::{RegionBoundary, RegionShadingLayer, ShadingRegion};
pub use time_axis::TimeAxisLayer;
pub use watermark::{WatermarkConfig, WatermarkLayer};

// Backwards compatibility type aliases
pub use auxiliary_bar::VolumeLayer;
pub use range_bar::{CandlestickConfig, CandlestickLayer};
pub use region_shading::SessionShadingLayer;

use crate::data::ChartData;
use crate::error::Result;
use crate::renderer::RenderContext;
use crate::style::ChartStyle;
use crate::theme::ChartTheme;
use crate::viewport::{Rect, Viewport};

/// Defines the coarse rendering order and default clipping region for a layer.
///
/// Layers are rendered from the smallest discriminant (background) to the
/// largest (HUD). Within each stage, [`Layer::z_order`] determines the exact
/// ordering so features can interleave when necessary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum LayerStage {
    /// Fills the entire render target (e.g. clear colour, full-screen background).
    ScreenBackground,
    /// Renders behind the main chart area (session shading, watermark).
    ChartBackground,
    /// Elements that sit underneath price data but inside the chart pane (gridlines).
    ChartUnderlay,
    /// Primary data plotted in the main chart pane (candles, bars).
    ChartMain,
    /// Technical overlays that should appear above price data (indicators).
    ChartIndicator,
    /// Bars/overlays that live exclusively in the volume pane.
    VolumePane,
    /// Foreground overlays sharing the main pane and axis surface (current price, annotations).
    ChartOverlay,
    /// Price-axis specific rendering (labels, markers).
    PriceAxis,
    /// Time-axis specific rendering (labels, markers).
    TimeAxis,
    /// Heads-up display drawn last (crosshair, tooltips).
    Hud,
}

impl LayerStage {
    /// Default clipping rectangle for the stage.
    pub fn default_clip_rect(self, viewport: &Viewport) -> Rect {
        match self {
            LayerStage::ScreenBackground => viewport.screen_rect,
            LayerStage::ChartBackground
            | LayerStage::ChartUnderlay
            | LayerStage::ChartMain
            | LayerStage::ChartIndicator
            | LayerStage::ChartOverlay => viewport.chart_content_rect(),
            LayerStage::VolumePane => viewport.volume_rect(),
            LayerStage::PriceAxis => viewport.price_axis_rect(),
            LayerStage::TimeAxis => viewport.time_axis_rect(),
            LayerStage::Hud => viewport.screen_rect,
        }
    }
}

/// Core trait for all chart layers
///
/// Layers are the building blocks of the chart rendering system. Each layer
/// is responsible for rendering a specific visual component and can be
/// independently updated and configured.
pub trait Layer: Send + Sync {
    /// Get the layer's name for debugging and identification
    fn name(&self) -> &str;

    /// Update the layer with new data
    ///
    /// This is called when the chart data changes. Layers should update
    /// their internal state and prepare for rendering.
    fn update(
        &mut self,
        data: &ChartData,
        viewport: &Viewport,
        theme: &ChartTheme,
        style: &ChartStyle,
    );

    /// Rendering stage for the layer. Stages define coarse ordering buckets.
    fn stage(&self) -> LayerStage {
        LayerStage::ChartMain
    }

    /// Default clipping rectangle for this layer.
    ///
    /// Implementers can override this when a layer spans multiple panes (e.g.
    /// the current-price line extends into the price axis).
    fn clip_rect(&self, viewport: &Viewport) -> Rect {
        self.stage().default_clip_rect(viewport)
    }

    /// Render the layer to the GPU context
    ///
    /// This is where the actual drawing happens. Layers should submit
    /// their geometry to the render context for GPU rendering.
    fn render(&self, context: &mut RenderContext, render_pass: &mut wgpu::RenderPass)
    -> Result<()>;

    /// Check if the layer needs to be rendered
    ///
    /// This allows for optimization by skipping layers that haven't changed
    /// or are not visible in the current viewport.
    fn needs_render(&self) -> bool {
        true // Default: always render
    }

    /// Get the layer's Z-order for depth sorting
    ///
    /// Lower values are rendered first (background), higher values
    /// are rendered last (foreground).
    fn z_order(&self) -> i32 {
        0 // Default: middle layer
    }

    /// Check if the layer is currently enabled
    fn is_enabled(&self) -> bool {
        true // Default: enabled
    }

    /// Enable or disable the layer
    fn set_enabled(&mut self, enabled: bool);

    /// Get layer configuration as a JSON-serializable value
    ///
    /// This allows saving and restoring layer configurations.
    fn get_config(&self) -> serde_json::Value {
        serde_json::Value::Null // Default: no configuration
    }

    /// Set layer configuration from a JSON value
    fn set_config(&mut self, _config: serde_json::Value) -> Result<()> {
        Ok(()) // Default: ignore configuration
    }
}

/// Layer manager for organizing and rendering multiple layers
pub struct LayerManager {
    layers: Vec<Box<dyn Layer>>,
    dirty: bool,
}

impl LayerManager {
    /// Create a new empty layer manager
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
            dirty: true,
        }
    }

    /// Add a layer to the manager
    pub fn add_layer(&mut self, layer: Box<dyn Layer>) {
        self.layers.push(layer);
        self.sort_layers();
        self.dirty = true;
    }

    /// Remove a layer by name
    pub fn remove_layer(&mut self, name: &str) -> Option<Box<dyn Layer>> {
        if let Some(index) = self.layers.iter().position(|layer| layer.name() == name) {
            self.dirty = true;
            Some(self.layers.remove(index))
        } else {
            None
        }
    }

    /// Get a mutable reference to a layer by name
    pub fn get_layer_mut(&mut self, name: &str) -> Option<&mut (dyn Layer + '_)> {
        for layer in &mut self.layers {
            if layer.name() == name {
                return Some(&mut **layer);
            }
        }
        None
    }

    /// Get an immutable reference to a layer by name
    pub fn get_layer(&self, name: &str) -> Option<&dyn Layer> {
        self.layers
            .iter()
            .find(|layer| layer.name() == name)
            .map(|layer| layer.as_ref())
    }

    /// Update all layers with new data
    pub fn update_all(
        &mut self,
        data: &ChartData,
        viewport: &Viewport,
        theme: &ChartTheme,
        style: &ChartStyle,
    ) {
        for layer in &mut self.layers {
            if layer.is_enabled() {
                layer.update(data, viewport, theme, style);
            }
        }
        self.dirty = true;
    }

    /// Render all enabled layers in Z-order
    pub fn render_all(
        &self,
        context: &mut RenderContext,
        render_pass: &mut wgpu::RenderPass,
    ) -> Result<()> {
        for layer in &self.layers {
            if layer.is_enabled() && layer.needs_render() {
                // Set stage priority for text grouping (stage enum discriminant + z_order for fine ordering)
                #[cfg(feature = "text-rendering")]
                {
                    let stage_priority = (layer.stage() as i32) * 1000 + layer.z_order();
                    context.set_stage_priority(stage_priority);
                }

                let clip_rect = {
                    let viewport = context.viewport();
                    layer.clip_rect(viewport)
                };

                // Clamp to positive extents before converting to u32.
                let x = clip_rect.x.max(0.0) as u32;
                let y = clip_rect.y.max(0.0) as u32;
                let width = clip_rect.width.max(0.0) as u32;
                let height = clip_rect.height.max(0.0) as u32;

                render_pass.set_scissor_rect(x, y, width, height);

                layer.render(context, render_pass)?;
                context.commit(render_pass)?;
            }
        }
        // Reset scissor rect to full screen after all layers are rendered
        let full_rect = context.viewport().screen_rect;
        render_pass.set_scissor_rect(
            full_rect.x as u32,
            full_rect.y as u32,
            full_rect.width as u32,
            full_rect.height as u32,
        );
        Ok(())
    }

    /// Check if any layer needs rendering
    pub fn needs_render(&self) -> bool {
        self.dirty
            || self
                .layers
                .iter()
                .any(|layer| layer.is_enabled() && layer.needs_render())
    }

    /// Mark the layer manager as clean (after rendering)
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Get the number of layers
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// Get the number of enabled layers
    pub fn enabled_layer_count(&self) -> usize {
        self.layers
            .iter()
            .filter(|layer| layer.is_enabled())
            .count()
    }

    /// List all layer names
    pub fn layer_names(&self) -> Vec<&str> {
        self.layers.iter().map(|layer| layer.name()).collect()
    }

    /// Enable or disable a layer by name
    pub fn set_layer_enabled(&mut self, name: &str, enabled: bool) -> bool {
        if let Some(layer) = self.get_layer_mut(name) {
            layer.set_enabled(enabled);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Clear all layers
    pub fn clear(&mut self) {
        self.layers.clear();
        self.dirty = true;
    }

    /// Sort layers by Z-order
    fn sort_layers(&mut self) {
        use std::cmp::Ordering;

        self.layers.sort_by(|a, b| match a.stage().cmp(&b.stage()) {
            Ordering::Equal => a.z_order().cmp(&b.z_order()),
            other => other,
        });
    }
}

impl Default for LayerManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper trait for creating common layer combinations
pub trait LayerBuilder {
    /// Create a basic range bar chart with grid and axes
    fn basic_range_chart() -> LayerManager {
        let mut manager = LayerManager::new();

        // Add layers in rendering order (background to foreground)
        manager.add_layer(Box::new(BackgroundLayer::new()));
        manager.add_layer(Box::new(RegionShadingLayer::new()));
        manager.add_layer(Box::new(GridLayer::new()));
        manager.add_layer(Box::new(RangeBarLayer::new()));
        manager.add_layer(Box::new(AuxiliaryBarLayer::new()));

        let mut price_config = CurrentPriceConfig::default();
        price_config.line_style = LineStyle::Dashed;
        price_config.line_width = 1.0;
        manager.add_layer(Box::new(CurrentPriceLayer::with_config(price_config)));
        manager.add_layer(Box::new(PriceAxisLayer::new()));
        manager.add_layer(Box::new(TimeAxisLayer::new()));
        manager.add_layer(Box::new(CrosshairLayer::new()));

        manager
    }

    /// Backwards compatibility alias
    fn basic_financial_chart() -> LayerManager {
        Self::basic_range_chart()
    }

    /// Create a chart with watermark
    fn chart_with_watermark(label: String) -> LayerManager {
        let mut manager = Self::basic_range_chart();
        manager.add_layer(Box::new(WatermarkLayer::new(label)));
        manager
    }
}

impl LayerBuilder for LayerManager {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ChartTheme;
    use crate::viewport::Viewport;

    // Mock layer for testing
    struct MockLayer {
        name: String,
        enabled: bool,
        z_order: i32,
        stage: LayerStage,
        render_count: std::sync::Arc<std::sync::Mutex<u32>>,
    }

    impl MockLayer {
        fn new(name: &str, z_order: i32) -> Self {
            Self::with_stage(name, z_order, LayerStage::ChartMain)
        }

        fn with_stage(name: &str, z_order: i32, stage: LayerStage) -> Self {
            Self {
                name: name.to_string(),
                enabled: true,
                z_order,
                stage,
                render_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
            }
        }
    }

    impl Layer for MockLayer {
        fn name(&self) -> &str {
            &self.name
        }

        fn update(
            &mut self,
            _data: &ChartData,
            _viewport: &Viewport,
            _theme: &ChartTheme,
            _style: &ChartStyle,
        ) {
            // Mock update
        }

        fn stage(&self) -> LayerStage {
            self.stage
        }

        fn render(
            &self,
            _context: &mut RenderContext,
            _render_pass: &mut wgpu::RenderPass,
        ) -> Result<()> {
            *self.render_count.lock().unwrap() += 1;
            Ok(())
        }

        fn z_order(&self) -> i32 {
            self.z_order
        }

        fn is_enabled(&self) -> bool {
            self.enabled
        }

        fn set_enabled(&mut self, enabled: bool) {
            self.enabled = enabled;
        }
    }

    #[test]
    fn test_layer_manager_basic_operations() {
        let mut manager = LayerManager::new();
        assert_eq!(manager.layer_count(), 0);

        // Add layers
        manager.add_layer(Box::new(MockLayer::new("layer1", 1)));
        manager.add_layer(Box::new(MockLayer::new("layer2", 0)));
        manager.add_layer(Box::new(MockLayer::new("layer3", 2)));

        assert_eq!(manager.layer_count(), 3);
        assert_eq!(manager.enabled_layer_count(), 3);

        // Check Z-order sorting
        let names = manager.layer_names();
        assert_eq!(names, vec!["layer2", "layer1", "layer3"]); // Sorted by z_order
    }

    #[test]
    fn test_layer_enable_disable() {
        let mut manager = LayerManager::new();
        manager.add_layer(Box::new(MockLayer::new("test", 0)));

        assert_eq!(manager.enabled_layer_count(), 1);

        manager.set_layer_enabled("test", false);
        assert_eq!(manager.enabled_layer_count(), 0);

        manager.set_layer_enabled("test", true);
        assert_eq!(manager.enabled_layer_count(), 1);
    }

    #[test]
    fn test_layer_removal() {
        let mut manager = LayerManager::new();
        manager.add_layer(Box::new(MockLayer::new("remove_me", 0)));
        manager.add_layer(Box::new(MockLayer::new("keep_me", 1)));

        assert_eq!(manager.layer_count(), 2);

        let removed = manager.remove_layer("remove_me");
        assert!(removed.is_some());
        assert_eq!(manager.layer_count(), 1);

        let not_found = manager.remove_layer("not_found");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_stage_sorting() {
        let mut manager = LayerManager::new();
        manager.add_layer(Box::new(MockLayer::with_stage("hud", 0, LayerStage::Hud)));
        manager.add_layer(Box::new(MockLayer::with_stage(
            "background",
            0,
            LayerStage::ScreenBackground,
        )));
        manager.add_layer(Box::new(MockLayer::with_stage(
            "indicator",
            0,
            LayerStage::ChartIndicator,
        )));

        let names = manager.layer_names();
        assert_eq!(names, vec!["background", "indicator", "hud"]);
    }
}
