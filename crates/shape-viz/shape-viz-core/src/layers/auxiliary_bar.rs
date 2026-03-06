//! Auxiliary bar rendering layer
//!
//! Renders auxiliary data as bars (volume, sample size, weight, etc.)
//! This is a pure geometry layer with no domain knowledge.

use crate::data::{ChartData, RangeSeries};
use crate::error::Result;
use crate::layers::{Layer, LayerStage};
use crate::renderer::RenderContext;
use crate::style::ChartStyle;
use crate::theme::{ChartTheme, Color};
use crate::viewport::{Rect, Viewport};

/// Configuration for auxiliary bars appearance
#[derive(Debug, Clone)]
pub struct AuxiliaryBarConfig {
    /// Height factor for auxiliary area (0.0-1.0, relative to chart height)
    pub height_factor: f32,
    /// Width factor for bars (0.0-1.0, relative to data spacing)
    pub bar_width_factor: f32,
    /// Minimum bar width in pixels
    pub min_width: f32,
    /// Maximum bar width in pixels
    pub max_width: f32,
}

impl Default for AuxiliaryBarConfig {
    fn default() -> Self {
        Self {
            height_factor: 0.2,    // Bottom 20% of chart
            bar_width_factor: 0.8, // Fill most of the bar width
            min_width: 1.0,
            max_width: 20.0,
        }
    }
}

/// Internal representation of a single bar
#[derive(Debug, Clone)]
struct BarGeometry {
    rect: Rect,
    color: Color,
}

/// Layer for rendering auxiliary data bars (volume, sample size, etc.)
#[derive(Debug)]
pub struct AuxiliaryBarLayer {
    enabled: bool,
    needs_render: bool,
    config: AuxiliaryBarConfig,
    bars: Vec<BarGeometry>,
    /// Optional data provided directly (timestamps, values, is_positive flags)
    auxiliary_data: Option<AuxiliaryData>,
}

/// Internal storage for auxiliary data when provided directly
#[derive(Debug, Clone)]
struct AuxiliaryData {
    timestamps: Vec<f64>,
    values: Vec<f64>,
    /// Optional positive/negative indicators for coloring
    is_positive: Option<Vec<bool>>,
}

impl AuxiliaryBarLayer {
    pub fn new() -> Self {
        Self {
            enabled: true,
            needs_render: true,
            config: AuxiliaryBarConfig::default(),
            bars: Vec::new(),
            auxiliary_data: None,
        }
    }

    pub fn with_config(config: AuxiliaryBarConfig) -> Self {
        let mut layer = Self::new();
        layer.config = config;
        layer
    }

    /// Set auxiliary data directly (for use without ChartData)
    pub fn set_auxiliary_data(
        &mut self,
        timestamps: Vec<f64>,
        values: Vec<f64>,
        is_positive: Option<Vec<bool>>,
    ) {
        self.auxiliary_data = Some(AuxiliaryData {
            timestamps,
            values,
            is_positive,
        });
        self.needs_render = true;
    }

    /// Calculate bars from internal auxiliary data
    fn calculate_bars_from_data(&mut self, viewport: &Viewport, theme: &ChartTheme) {
        self.bars.clear();

        let data = match &self.auxiliary_data {
            Some(d) => d,
            None => return,
        };

        if data.timestamps.is_empty() || data.values.is_empty() {
            return;
        }

        let count = data.timestamps.len().min(data.values.len());

        // Find max value
        let max_value = data
            .values
            .iter()
            .take(count)
            .fold(0.0f64, |a, &b| a.max(b));
        if max_value <= 0.0 {
            return;
        }

        let chart_rect = viewport.chart_content_rect();
        let volume_rect = viewport.volume_rect();
        let time_duration = viewport.chart_bounds.time_duration().num_seconds() as f32;
        if time_duration <= 0.0 {
            return;
        }

        let time_scale = chart_rect.width / time_duration;
        let time_spacing = if count > 1 {
            let first = data.timestamps[0] as f32;
            let last = data.timestamps[count - 1] as f32;
            (last - first) / (count - 1) as f32
        } else {
            60.0
        };

        let screen_time_width = time_spacing * time_scale;
        let bar_width = (screen_time_width * self.config.bar_width_factor)
            .max(self.config.min_width)
            .min(self.config.max_width);
        let half_width = bar_width * 0.5;
        let time_start = viewport.chart_bounds.time_start.timestamp() as f32;

        for i in 0..count {
            let timestamp = data.timestamps[i];
            let value = data.values[i];

            let delta = timestamp as f32 - time_start;
            let x_center = chart_rect.x + delta * time_scale;
            let value_ratio = (value / max_value).clamp(0.0, 1.0) as f32;
            let desired_height = (volume_rect.height * 0.95) * value_ratio;
            let final_height = desired_height.max(2.0);

            let rect = Rect::new(
                x_center - half_width,
                volume_rect.y + volume_rect.height - final_height,
                bar_width,
                final_height,
            );

            let is_positive = data
                .is_positive
                .as_ref()
                .map(|v| v.get(i).copied().unwrap_or(true))
                .unwrap_or(true);
            let color = if is_positive {
                theme.colors.volume_bullish
            } else {
                theme.colors.volume_bearish
            };

            self.bars.push(BarGeometry { rect, color });
        }

        self.needs_render = !self.bars.is_empty();
    }

    /// Calculate bars from a RangeSeries
    fn _calculate_bars_from_series<S: RangeSeries + ?Sized>(
        &mut self,
        series: &S,
        data: &ChartData,
        viewport: &Viewport,
        theme: &ChartTheme,
    ) {
        self.bars.clear();

        let (start_idx, end_idx) = match data.visible_indices() {
            Some((start, end)) => (start, end),
            None => return,
        };

        if start_idx >= end_idx {
            return;
        }

        // Find max auxiliary value in visible range
        let mut max_value = 0.0f64;
        for i in start_idx..end_idx {
            if let Some(aux) = series.get_auxiliary(i) {
                if aux > max_value {
                    max_value = aux;
                }
            }
        }

        if max_value <= 0.0 {
            return;
        }

        let chart_rect = viewport.chart_content_rect();
        let volume_rect = viewport.volume_rect();
        let time_duration = viewport.chart_bounds.time_duration().num_seconds() as f32;
        if time_duration <= 0.0 {
            return;
        }

        let time_scale = chart_rect.width / time_duration;
        let count = end_idx - start_idx;

        let time_spacing = if count > 1 {
            let first = series.get_x(start_idx) as f32;
            let last = series.get_x(end_idx - 1) as f32;
            (last - first) / (count - 1) as f32
        } else {
            60.0
        };

        let screen_time_width = time_spacing * time_scale;
        let bar_width = (screen_time_width * self.config.bar_width_factor)
            .max(self.config.min_width)
            .min(self.config.max_width);
        let half_width = bar_width * 0.5;
        let time_start = viewport.chart_bounds.time_start.timestamp() as f32;

        for i in start_idx..end_idx {
            let aux = match series.get_auxiliary(i) {
                Some(v) => v,
                None => continue,
            };
            let (start_val, _, _, end_val) = series.get_range(i);
            let timestamp_f64 = series.get_x(i);

            let delta = timestamp_f64 as f32 - time_start;
            let x_center = chart_rect.x + delta * time_scale;
            let value_ratio = (aux / max_value).clamp(0.0, 1.0) as f32;
            let desired_height = (volume_rect.height * 0.95) * value_ratio;
            let final_height = desired_height.max(2.0);

            let rect = Rect::new(
                x_center - half_width,
                volume_rect.y + volume_rect.height - final_height,
                bar_width,
                final_height,
            );

            // Use range end >= start to determine color
            let is_positive = end_val >= start_val;
            let color = if is_positive {
                theme.colors.volume_bullish
            } else {
                theme.colors.volume_bearish
            };

            self.bars.push(BarGeometry { rect, color });
        }

        self.needs_render = !self.bars.is_empty();
    }
}

impl Default for AuxiliaryBarLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Layer for AuxiliaryBarLayer {
    fn name(&self) -> &str {
        "AuxiliaryBar"
    }

    fn stage(&self) -> LayerStage {
        LayerStage::VolumePane
    }

    fn update(
        &mut self,
        _data: &ChartData,
        viewport: &Viewport,
        theme: &ChartTheme,
        _style: &ChartStyle,
    ) {
        // Use internal data if set
        if self.auxiliary_data.is_some() {
            self.calculate_bars_from_data(viewport, theme);
            return;
        }

        // Otherwise try to calculate from ChartData
        // Note: ChartData integration with RangeSeries requires the series to implement
        // the trait - this will be connected when wire protocol integration is complete
        self.bars.clear();
        self.needs_render = false;
    }

    fn render(
        &self,
        context: &mut RenderContext,
        _render_pass: &mut wgpu::RenderPass,
    ) -> Result<()> {
        if !self.enabled || self.bars.is_empty() {
            return Ok(());
        }

        let volume_rect = context.viewport().volume_rect();
        let (background_darken, baseline_alpha) = {
            let s = &context.style().volume;
            (s.background_darken, s.baseline_alpha)
        };
        let background = context
            .theme()
            .colors
            .chart_background
            .darken(background_darken);
        context.draw_rect(volume_rect, background);

        context.draw_line(
            [volume_rect.x, volume_rect.y],
            [volume_rect.x + volume_rect.width, volume_rect.y],
            context.theme().colors.grid_major.with_alpha(baseline_alpha),
            1.0,
        );

        for bar in &self.bars {
            context.draw_rect(bar.rect, bar.color);
        }
        Ok(())
    }

    fn needs_render(&self) -> bool {
        self.needs_render
    }

    fn z_order(&self) -> i32 {
        -50 // Background
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.needs_render = true;
    }
}

// Type aliases for backwards compatibility
pub type VolumeLayer = AuxiliaryBarLayer;
pub type VolumeConfig = AuxiliaryBarConfig;
