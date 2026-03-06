//! Main chart API and configuration

use crate::data::ChartData;
use crate::error::{ChartError, Result};
use crate::layers::{Layer, LayerBuilder, LayerManager};
use crate::renderer::{GpuRenderer, RenderContext};
use crate::style::ChartStyle;
use crate::theme::ChartTheme;
use crate::viewport::{ChartBounds, Rect, Viewport};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Configuration for chart creation and behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartConfig {
    /// Chart dimensions in pixels
    pub width: u32,
    pub height: u32,

    /// Chart theme
    pub theme: ChartTheme,

    /// Chart styling parameters
    pub style: ChartStyle,

    /// Auto-fit data to viewport
    pub auto_fit: bool,

    /// Enable anti-aliasing
    pub anti_aliasing: bool,

    /// Maximum frames per second for animations
    pub max_fps: f32,

    /// Padding around chart content (percentage of viewport)
    pub padding: f32,

    /// Enable GPU acceleration
    pub gpu_acceleration: bool,
}

impl Default for ChartConfig {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            theme: ChartTheme::default(),
            style: crate::style::ChartStyle::default(),
            auto_fit: true,
            anti_aliasing: true,
            max_fps: 60.0,
            padding: 0.02,
            gpu_acceleration: true,
        }
    }
}

/// Main chart instance that manages rendering and data
pub struct Chart {
    config: ChartConfig,
    renderer: Option<GpuRenderer>,
    render_context: Option<RenderContext>,
    layer_manager: LayerManager,
    data: Option<ChartData>,
    viewport: Viewport,
    dirty: bool,
}

impl Chart {
    /// Create a new chart with the given configuration
    pub async fn new(config: ChartConfig) -> Result<Self> {
        // Create viewport that represents the full rendering area
        // The chart content area will be inset to make room for axes
        let full_rect = Rect::new(0.0, 0.0, config.width as f32, config.height as f32);

        let default_bounds = ChartBounds::new(
            Utc::now() - chrono::Duration::hours(24),
            Utc::now(),
            0.0,
            100.0,
        )?;
        let viewport = Viewport::new(full_rect, default_bounds, config.style.layout.clone());

        // Initialize GPU renderer if enabled
        let renderer = if config.gpu_acceleration {
            Some(GpuRenderer::new_offscreen(config.width, config.height).await?)
        } else {
            None
        };

        // Create render context
        let render_context = if let Some(ref renderer) = renderer {
            let (device, queue) = renderer.device_and_queue();
            Some(RenderContext::new(
                device,
                queue,
                viewport.clone(),
                config.theme.clone(),
                config.style.clone(),
            ))
        } else {
            None
        };

        Ok(Self {
            config,
            renderer,
            render_context,
            layer_manager: LayerManager::new(),
            data: None,
            viewport,
            dirty: true,
        })
    }

    /// Create a basic financial chart with default layers
    pub async fn new_financial(config: ChartConfig) -> Result<Self> {
        let mut chart = Self::new(config).await?;
        chart.layer_manager = LayerManager::basic_financial_chart();
        chart.dirty = true;
        Ok(chart)
    }

    /// Set chart data
    pub fn set_data(&mut self, data: ChartData) -> Result<()> {
        // Auto-fit viewport to data if enabled
        if self.config.auto_fit {
            if let Some(time_range) = data.time_range() {
                if let Some((min_price, max_price)) = data.y_bounds() {
                    // Just use tight bounds without forcing to nice numbers
                    let price_range = max_price - min_price;
                    let padding = price_range * self.config.padding as f64;

                    // Simple padded bounds
                    let padded_min = min_price - padding;
                    let padded_max = max_price + padding;

                    let chart_bounds =
                        ChartBounds::new(time_range.start, time_range.end, padded_min, padded_max)?;

                    self.viewport.set_chart_bounds(chart_bounds);
                }
            }
        }

        self.data = Some(data);
        self.dirty = true;
        Ok(())
    }

    /// Get current chart data
    pub fn data(&self) -> Option<&ChartData> {
        self.data.as_ref()
    }

    /// Add a layer to the chart
    pub fn add_layer(&mut self, layer: Box<dyn Layer>) {
        self.layer_manager.add_layer(layer);
        self.dirty = true;
    }

    /// Remove a layer by name
    pub fn remove_layer(&mut self, name: &str) -> Option<Box<dyn Layer>> {
        self.dirty = true;
        self.layer_manager.remove_layer(name)
    }

    /// Get a mutable reference to a layer
    pub fn get_layer_mut(&mut self, name: &str) -> Option<&mut (dyn Layer + '_)> {
        self.layer_manager.get_layer_mut(name)
    }

    /// Enable or disable a layer
    pub fn set_layer_enabled(&mut self, name: &str, enabled: bool) -> bool {
        let result = self.layer_manager.set_layer_enabled(name, enabled);
        if result {
            self.dirty = true;
        }
        result
    }

    /// Get current viewport
    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    /// Set viewport bounds
    pub fn set_viewport_bounds(&mut self, bounds: ChartBounds) {
        self.viewport.set_chart_bounds(bounds);
        self.dirty = true;
    }

    /// Pan the viewport by screen pixels
    pub fn pan(&mut self, delta_x: f32, delta_y: f32) {
        self.viewport.pan(glam::Vec2::new(delta_x, delta_y));
        self.dirty = true;
    }

    /// Zoom the viewport around a center point
    pub fn zoom(&mut self, center_x: f32, center_y: f32, zoom_factor: f32) {
        self.viewport
            .zoom(glam::Vec2::new(center_x, center_y), zoom_factor);
        self.dirty = true;
    }

    /// Reset viewport to fit all data
    pub fn fit_to_data(&mut self) -> Result<()> {
        if let Some(ref data) = self.data {
            if let Some(time_range) = data.time_range() {
                if let Some((min_price, max_price)) = data.y_bounds() {
                    let price_range = max_price - min_price;
                    let price_padding = price_range * self.config.padding as f64;

                    let chart_bounds = ChartBounds::new(
                        time_range.start,
                        time_range.end,
                        min_price - price_padding,
                        max_price + price_padding,
                    )?;

                    self.viewport.set_chart_bounds(chart_bounds);
                    self.dirty = true;
                }
            }
        }
        Ok(())
    }

    /// Update chart configuration
    pub fn set_config(&mut self, config: ChartConfig) {
        self.viewport.set_layout_style(config.style.layout.clone());
        self.config = config;
        self.dirty = true;
    }

    /// Get current configuration
    pub fn config(&self) -> &ChartConfig {
        &self.config
    }

    /// Set chart theme
    pub fn set_theme(&mut self, theme: ChartTheme) {
        self.config.theme = theme;
        self.dirty = true;
    }

    /// Check if chart needs to be re-rendered
    pub fn needs_render(&self) -> bool {
        self.dirty || self.layer_manager.needs_render()
    }

    /// Render the chart and return RGBA image data
    pub async fn render(&mut self) -> Result<Vec<u8>> {
        // Check if we have necessary components
        let renderer = self
            .renderer
            .as_ref()
            .ok_or_else(|| ChartError::internal("No renderer available"))?;

        let render_context = self
            .render_context
            .as_mut()
            .ok_or_else(|| ChartError::internal("No render context available"))?;

        // Update render context
        render_context.update(
            self.viewport.clone(),
            self.config.theme.clone(),
            self.config.style.clone(),
        );

        // Update all layers if we have data
        if let Some(ref data) = self.data {
            self.layer_manager.update_all(
                data,
                &self.viewport,
                &self.config.theme,
                &self.config.style,
            );
        }

        // Clear previous frame
        render_context.clear();

        // Execute GPU render
        let image_data = renderer
            .render(
                render_context,
                &self.layer_manager,
                self.config.theme.colors.background,
            )
            .await?;

        // Mark as clean
        self.dirty = false;
        self.layer_manager.mark_clean();

        Ok(image_data)
    }

    /// Get chart dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Resize the chart
    pub async fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width != self.config.width || height != self.config.height {
            self.config.width = width;
            self.config.height = height;

            // Update viewport screen rect to full size
            let full_rect = Rect::new(0.0, 0.0, width as f32, height as f32);
            self.viewport.set_screen_rect(full_rect);

            // Recreate renderer with new dimensions
            if self.config.gpu_acceleration {
                self.renderer = Some(GpuRenderer::new_offscreen(width, height).await?);

                // Update render context
                if let Some(ref renderer) = self.renderer {
                    let (device, queue) = renderer.device_and_queue();
                    self.render_context = Some(RenderContext::new(
                        device,
                        queue,
                        self.viewport.clone(),
                        self.config.theme.clone(),
                        self.config.style.clone(),
                    ));
                }
            }

            self.dirty = true;
        }
        Ok(())
    }

    /// Convert screen coordinates to chart coordinates
    pub fn screen_to_chart(&self, screen_x: f32, screen_y: f32) -> glam::Vec2 {
        self.viewport
            .screen_to_chart(glam::Vec2::new(screen_x, screen_y))
    }

    /// Convert chart coordinates to screen coordinates
    pub fn chart_to_screen(&self, chart_x: f32, chart_y: f32) -> glam::Vec2 {
        self.viewport
            .chart_to_screen(glam::Vec2::new(chart_x, chart_y))
    }

    /// Find data point at screen coordinates
    /// Returns: (index, timestamp, start, max, min, end, auxiliary)
    pub fn hit_test(
        &self,
        screen_x: f32,
        screen_y: f32,
    ) -> Option<(usize, DateTime<Utc>, f64, f64, f64, f64, f64)> {
        let data = self.data.as_ref()?;
        let chart_pos = self.screen_to_chart(screen_x, screen_y);

        // Convert chart X coordinate (timestamp) to find nearest data point
        let index = data.main_series.find_index(chart_pos.x as f64)?;
        let time_val = data.main_series.get_x(index);
        let time = DateTime::from_timestamp(time_val as i64, 0)?;

        // Use Y value for simple series
        let val = data.main_series.get_y(index);
        Some((index, time, val, val, val, val, 0.0))
    }

    /// Get visible time range as timestamps
    pub fn visible_time_range(&self) -> (i64, i64) {
        self.viewport.visible_time_range()
    }

    /// Get visible price range
    pub fn visible_price_range(&self) -> (f64, f64) {
        self.viewport.visible_price_range()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{ChartData, RangeSeries, Series};
    use chrono::TimeZone;
    use std::any::Any;

    fn should_skip_gpu(err: &ChartError) -> bool {
        matches!(err, ChartError::Internal(message)
            if message.contains("No suitable graphics adapter"))
    }

    /// Mock range series for testing (no external dependencies)
    #[derive(Debug, Clone)]
    struct MockRangeSeries {
        name: String,
        timestamps: Vec<i64>,
        ranges: Vec<(f64, f64, f64, f64)>, // (start, max, min, end)
        auxiliary: Vec<f64>,
    }

    impl MockRangeSeries {
        fn new(name: &str, count: usize) -> Self {
            let base_time = chrono::Utc
                .with_ymd_and_hms(2024, 1, 1, 9, 0, 0)
                .unwrap()
                .timestamp();
            let mut timestamps = Vec::with_capacity(count);
            let mut ranges = Vec::with_capacity(count);
            let mut auxiliary = Vec::with_capacity(count);

            let mut last_end = 100.0;
            for i in 0..count {
                timestamps.push(base_time + (i as i64 * 3600)); // hourly
                let start = last_end;
                let movement = (i as f64 * 0.1).sin() * 5.0;
                let end = start + movement;
                let max = start.max(end) + 2.0;
                let min = start.min(end) - 2.0;
                ranges.push((start, max, min, end));
                auxiliary.push(1000.0 + (i as f64 * 100.0));
                last_end = end;
            }

            Self {
                name: name.to_string(),
                timestamps,
                ranges,
                auxiliary,
            }
        }
    }

    impl Series for MockRangeSeries {
        fn name(&self) -> &str {
            &self.name
        }

        fn len(&self) -> usize {
            self.timestamps.len()
        }

        fn get_x(&self, index: usize) -> f64 {
            self.timestamps[index] as f64
        }

        fn get_y(&self, index: usize) -> f64 {
            self.ranges[index].3 // end value
        }

        fn get_x_range(&self) -> (f64, f64) {
            if self.timestamps.is_empty() {
                return (0.0, 1.0);
            }
            (
                self.timestamps[0] as f64,
                self.timestamps[self.timestamps.len() - 1] as f64,
            )
        }

        fn get_y_range(&self, _x_min: f64, _x_max: f64) -> (f64, f64) {
            let mut min = f64::INFINITY;
            let mut max = f64::NEG_INFINITY;
            for (_, hi, lo, _) in &self.ranges {
                min = min.min(*lo);
                max = max.max(*hi);
            }
            if min.is_infinite() {
                (0.0, 100.0)
            } else {
                (min, max)
            }
        }

        fn find_index(&self, x: f64) -> Option<usize> {
            let target = x as i64;
            match self.timestamps.binary_search(&target) {
                Ok(idx) => Some(idx),
                Err(idx) => Some(idx.min(self.len().saturating_sub(1))),
            }
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    impl RangeSeries for MockRangeSeries {
        fn get_range(&self, index: usize) -> (f64, f64, f64, f64) {
            self.ranges[index]
        }

        fn get_auxiliary(&self, index: usize) -> Option<f64> {
            Some(self.auxiliary[index])
        }
    }

    #[tokio::test]
    async fn test_chart_creation() {
        let config = ChartConfig::default();
        let chart = match Chart::new(config).await {
            Ok(chart) => chart,
            Err(err) if should_skip_gpu(&err) => {
                eprintln!("skipping test_chart_creation: {err}");
                return;
            }
            Err(err) => panic!("chart creation failed: {err}"),
        };
        assert_eq!(chart.dimensions(), (800, 600));
        assert!(chart.needs_render()); // Should be dirty initially
    }

    #[tokio::test]
    async fn test_chart_with_data() {
        let config = ChartConfig::default();
        let mut chart = match Chart::new(config).await {
            Ok(chart) => chart,
            Err(err) if should_skip_gpu(&err) => {
                eprintln!("skipping test_chart_with_data: {err}");
                return;
            }
            Err(err) => panic!("chart creation failed: {err}"),
        };

        // Create test data using mock series
        let series = MockRangeSeries::new("TEST", 10);
        let chart_data = ChartData::new(Box::new(series));
        let result = chart.set_data(chart_data);
        assert!(result.is_ok());

        assert!(chart.data().is_some());
        assert!(chart.needs_render());
    }

    #[tokio::test]
    async fn test_chart_viewport_operations() {
        let config = ChartConfig::default();
        let mut chart = match Chart::new(config).await {
            Ok(chart) => chart,
            Err(err) if should_skip_gpu(&err) => {
                eprintln!("skipping test_chart_viewport_operations: {err}");
                return;
            }
            Err(err) => panic!("chart creation failed: {err}"),
        };

        let original_bounds = chart.viewport().chart_bounds;

        // Test pan
        chart.pan(10.0, 20.0);
        assert!(chart.needs_render());

        // Test zoom
        chart.zoom(400.0, 300.0, 2.0);
        assert!(chart.needs_render());

        // Bounds should have changed
        assert_ne!(
            chart.viewport().chart_bounds.time_start,
            original_bounds.time_start
        );
    }

    #[test]
    fn test_config_defaults() {
        let config = ChartConfig::default();
        assert_eq!(config.width, 800);
        assert_eq!(config.height, 600);
        assert!(config.auto_fit);
        assert!(config.anti_aliasing);
        assert!(config.gpu_acceleration);
        assert_eq!(config.max_fps, 60.0);
    }

    #[tokio::test]
    async fn test_full_chart_render() {
        let config = ChartConfig::default();
        let mut chart = match Chart::new_financial(config).await {
            Ok(chart) => chart,
            Err(err) if should_skip_gpu(&err) => {
                eprintln!("skipping test_full_chart_render: {err}");
                return;
            }
            Err(err) => panic!("chart creation failed: {err}"),
        };

        // Create test data using mock series
        let series = MockRangeSeries::new("TEST/USD", 50);
        let chart_data = ChartData::new(Box::new(series));
        chart.set_data(chart_data).unwrap();

        // Verify chart is ready to render
        assert!(chart.needs_render());
        assert!(chart.data().is_some());

        // Render the chart
        let image_data = chart.render().await.unwrap();

        // Verify image data is correct size
        assert_eq!(image_data.len(), 800 * 600 * 4); // RGBA

        // Verify it's not just a blank image (should have some color)
        let non_zero_pixels = image_data.iter().any(|&byte| byte != 0);
        assert!(non_zero_pixels, "Chart should contain rendered pixels");

        println!(
            "Chart rendered successfully with {} bytes of RGBA data",
            image_data.len()
        );
    }
}
