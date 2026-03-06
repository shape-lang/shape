//! Range bar rendering layer
//!
//! Renders range data as bars (candlesticks, box plots, error bars, etc.)
//! This is a pure geometry layer with no domain knowledge.

use crate::data::{ChartData, RangeSeries};
use crate::error::Result;
use crate::layers::{Layer, LayerStage};
use crate::renderer::RenderContext;
use crate::style::ChartStyle;
use crate::theme::ChartTheme;
use crate::viewport::{Rect, Viewport};

/// Visual style for range bars
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RangeBarStyle {
    /// Traditional candlestick style (body + wicks)
    #[default]
    Candlestick,
    /// Box plot style (box + whiskers)
    BoxPlot,
    /// Error bar style (center line + error bars)
    ErrorBar,
    /// Filled area between min and max
    RangeArea,
}

/// Configuration for range bar appearance
#[derive(Debug, Clone)]
pub struct RangeBarConfig {
    /// Visual style
    pub style: RangeBarStyle,
    /// Width factor for bar bodies (0.0-1.0, relative to time spacing)
    pub body_width_factor: f32,
    /// Width of lines (wicks, whiskers) in pixels
    pub line_width: f32,
    /// Minimum bar width in pixels
    pub min_width: f32,
    /// Maximum bar width in pixels
    pub max_width: f32,
}

impl Default for RangeBarConfig {
    fn default() -> Self {
        Self {
            style: RangeBarStyle::default(),
            body_width_factor: 0.7,
            line_width: 1.5,
            min_width: 1.5,
            max_width: 40.0,
        }
    }
}

/// Cached geometry for a single range bar
#[derive(Debug, Clone)]
struct RangeBarGeometry {
    pub body_rect: Rect,
    pub line_top: (f32, f32, f32, f32), // (x1, y1, x2, y2)
    pub line_bottom: (f32, f32, f32, f32),
    /// True if end >= start (e.g., close >= open for candlesticks)
    pub is_positive: bool,
    /// True if the range is very small relative to overall range
    pub is_neutral: bool,
}

/// Layer for rendering range bar charts (candlesticks, box plots, etc.)
#[derive(Debug)]
pub struct RangeBarLayer {
    enabled: bool,
    needs_render: bool,
    config: RangeBarConfig,
    cached_bars: Vec<RangeBarGeometry>,
    last_viewport_hash: u64,
    /// Optional range series provided directly (for non-ChartData usage)
    range_data: Option<RangeData>,
}

/// Internal storage for range data when provided directly
#[derive(Debug, Clone)]
struct RangeData {
    timestamps: Vec<f64>,
    ranges: Vec<(f64, f64, f64, f64)>, // (start, max, min, end)
    _auxiliary: Option<Vec<f64>>,
}

impl RangeBarLayer {
    pub fn new() -> Self {
        Self {
            enabled: true,
            needs_render: true,
            config: RangeBarConfig::default(),
            cached_bars: Vec::new(),
            last_viewport_hash: 0,
            range_data: None,
        }
    }

    pub fn with_config(config: RangeBarConfig) -> Self {
        Self {
            config,
            ..Self::new()
        }
    }

    pub fn with_style(style: RangeBarStyle) -> Self {
        Self {
            config: RangeBarConfig {
                style,
                ..Default::default()
            },
            ..Self::new()
        }
    }

    /// Set range data directly (for use without ChartData)
    pub fn set_range_data(
        &mut self,
        timestamps: Vec<f64>,
        ranges: Vec<(f64, f64, f64, f64)>,
        auxiliary: Option<Vec<f64>>,
    ) {
        self.range_data = Some(RangeData {
            timestamps,
            ranges,
            _auxiliary: auxiliary,
        });
        self.last_viewport_hash = 0; // Force recalculation
    }

    /// Calculate viewport hash for cache invalidation
    fn viewport_hash(viewport: &Viewport) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();

        viewport.screen_rect.x.to_bits().hash(&mut hasher);
        viewport.screen_rect.y.to_bits().hash(&mut hasher);
        viewport.screen_rect.width.to_bits().hash(&mut hasher);
        viewport.screen_rect.height.to_bits().hash(&mut hasher);
        viewport
            .chart_bounds
            .time_start
            .timestamp()
            .hash(&mut hasher);
        viewport.chart_bounds.time_end.timestamp().hash(&mut hasher);
        viewport.chart_bounds.price_min.to_bits().hash(&mut hasher);
        viewport.chart_bounds.price_max.to_bits().hash(&mut hasher);

        hasher.finish()
    }

    /// Calculate geometry from internal range data
    fn calculate_geometry_from_data(&mut self, viewport: &Viewport, style: &ChartStyle) {
        self.cached_bars.clear();

        let data = match &self.range_data {
            Some(d) => d,
            None => return,
        };

        if data.timestamps.is_empty() {
            return;
        }

        let count = data.timestamps.len();

        // Calculate average time spacing for width calculation
        let time_spacing = if count > 1 {
            let first_time = data.timestamps[0] as f32;
            let last_time = data.timestamps[count - 1] as f32;
            (last_time - first_time) / (count - 1) as f32
        } else {
            3600.0
        };

        let content_rect = viewport.layout.main_panel;
        let time_scale =
            content_rect.width / (viewport.chart_bounds.time_duration().num_seconds() as f32);
        let screen_time_width = time_spacing * time_scale;

        let body_width = (screen_time_width * self.config.body_width_factor)
            .max(self.config.min_width)
            .min(self.config.max_width);

        let half_body_width = body_width * 0.5;

        for i in 0..count {
            let timestamp = data.timestamps[i];
            let (start_val, max_val, min_val, end_val) = data.ranges[i];

            // Map timestamp to screen X
            let delta_sec =
                (timestamp - viewport.chart_bounds.time_start.timestamp() as f64) as f32;
            let x = content_rect.x + delta_sec * time_scale;

            let y_start = viewport.chart_to_screen_y(start_val as f32);
            let y_max = viewport.chart_to_screen_y(max_val as f32);
            let y_min = viewport.chart_to_screen_y(min_val as f32);
            let y_end = viewport.chart_to_screen_y(end_val as f32);

            let is_positive = end_val >= start_val;
            let range = (max_val - min_val).abs().max(1e-9);
            let body_span = (end_val - start_val).abs();
            let is_neutral = (body_span / range) < 0.05;

            let body_top = y_start.min(y_end);
            let body_bottom = y_start.max(y_end);
            let body_height = body_bottom - body_top;

            let min_body_height = style.candles.min_body_height;
            let adjusted_body_height = body_height.max(min_body_height);
            let body_y = if body_height < min_body_height {
                (body_top + body_bottom - adjusted_body_height) * 0.5
            } else {
                body_top
            };

            let body_rect = Rect::new(
                x - half_body_width,
                body_y,
                body_width,
                adjusted_body_height,
            );

            let line_top = if y_max < body_top {
                (x, y_max, x, body_top)
            } else {
                (x, y_max, x, y_max)
            };

            let line_bottom = if y_min > body_bottom {
                (x, body_bottom, x, y_min)
            } else {
                (x, y_min, x, y_min)
            };

            self.cached_bars.push(RangeBarGeometry {
                body_rect,
                line_top,
                line_bottom,
                is_positive,
                is_neutral,
            });
        }
    }

    /// Calculate geometry from a RangeSeries
    fn _calculate_geometry_from_series<S: RangeSeries + ?Sized>(
        &mut self,
        series: &S,
        data: &ChartData,
        viewport: &Viewport,
        _theme: &ChartTheme,
        style: &ChartStyle,
    ) {
        self.cached_bars.clear();

        let (start_idx, end_idx) = match data.visible_indices() {
            Some((start, end)) => (start, end),
            None => (0, series.len()),
        };

        if start_idx >= end_idx {
            return;
        }

        let count = end_idx - start_idx;

        let time_spacing = if count > 1 {
            let first_time = series.get_x(start_idx) as f32;
            let last_time = series.get_x(end_idx - 1) as f32;
            (last_time - first_time) / (count - 1) as f32
        } else {
            3600.0
        };

        let content_rect = viewport.layout.main_panel;
        let time_scale =
            content_rect.width / (viewport.chart_bounds.time_duration().num_seconds() as f32);
        let screen_time_width = time_spacing * time_scale;

        let body_width = (screen_time_width * self.config.body_width_factor)
            .max(self.config.min_width)
            .min(self.config.max_width);

        let half_body_width = body_width * 0.5;

        for i in start_idx..end_idx {
            let (start_val, max_val, min_val, end_val) = series.get_range(i);
            let timestamp_f64 = series.get_x(i);

            let delta_sec =
                (timestamp_f64 - viewport.chart_bounds.time_start.timestamp() as f64) as f32;
            let x = content_rect.x + delta_sec * time_scale;

            let y_start = viewport.chart_to_screen_y(start_val as f32);
            let y_max = viewport.chart_to_screen_y(max_val as f32);
            let y_min = viewport.chart_to_screen_y(min_val as f32);
            let y_end = viewport.chart_to_screen_y(end_val as f32);

            let is_positive = end_val >= start_val;
            let range = (max_val - min_val).abs().max(1e-9);
            let body_span = (end_val - start_val).abs();
            let is_neutral = (body_span / range) < 0.05;

            let body_top = y_start.min(y_end);
            let body_bottom = y_start.max(y_end);
            let body_height = body_bottom - body_top;

            let min_body_height = style.candles.min_body_height;
            let adjusted_body_height = body_height.max(min_body_height);
            let body_y = if body_height < min_body_height {
                (body_top + body_bottom - adjusted_body_height) * 0.5
            } else {
                body_top
            };

            let body_rect = Rect::new(
                x - half_body_width,
                body_y,
                body_width,
                adjusted_body_height,
            );

            let line_top = if y_max < body_top {
                (x, y_max, x, body_top)
            } else {
                (x, y_max, x, y_max)
            };

            let line_bottom = if y_min > body_bottom {
                (x, body_bottom, x, y_min)
            } else {
                (x, y_min, x, y_min)
            };

            self.cached_bars.push(RangeBarGeometry {
                body_rect,
                line_top,
                line_bottom,
                is_positive,
                is_neutral,
            });
        }
    }

    /// Render cached geometry
    fn render_cached_geometry(
        &self,
        context: &mut RenderContext,
        theme: &ChartTheme,
    ) -> Result<()> {
        let content_rect = context.viewport().chart_content_rect();

        for bar in &self.cached_bars {
            if bar.body_rect.x + bar.body_rect.width < content_rect.x
                || bar.body_rect.x > content_rect.x + content_rect.width
                || bar.body_rect.y + bar.body_rect.height < content_rect.y
                || bar.body_rect.y > content_rect.y + content_rect.height
            {
                continue;
            }

            let body_color = if bar.is_neutral {
                theme.colors.candle_doji
            } else if bar.is_positive {
                theme.colors.candle_bullish
            } else {
                theme.colors.candle_bearish
            };

            let line_color = if bar.is_neutral {
                theme.colors.wick_color
            } else if bar.is_positive {
                theme.colors.wick_bullish
            } else {
                theme.colors.wick_bearish
            };

            context.draw_rect(bar.body_rect, body_color);

            let (x1, y1, x2, y2) = bar.line_top;
            if (y2 - y1).abs() > 0.1 {
                context.draw_line([x1, y1], [x2, y2], line_color, self.config.line_width);
            }

            let (x1, y1, x2, y2) = bar.line_bottom;
            if (y2 - y1).abs() > 0.1 {
                context.draw_line([x1, y1], [x2, y2], line_color, self.config.line_width);
            }
        }

        Ok(())
    }
}

impl Default for RangeBarLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Layer for RangeBarLayer {
    fn name(&self) -> &str {
        "RangeBar"
    }

    fn stage(&self) -> LayerStage {
        LayerStage::ChartMain
    }

    fn update(
        &mut self,
        _data: &ChartData,
        viewport: &Viewport,
        _theme: &ChartTheme,
        style: &ChartStyle,
    ) {
        self.config.body_width_factor = style.candles.body_width_factor;
        self.config.line_width = style.candles.wick_width;
        self.config.min_width = style.candles.min_body_width;
        self.config.max_width = style.candles.max_body_width;

        let viewport_hash = Self::viewport_hash(viewport);

        if viewport_hash != self.last_viewport_hash {
            // Use internal range data if set, otherwise try to use ChartData's main series
            if self.range_data.is_some() {
                self.calculate_geometry_from_data(viewport, style);
            }
            // Note: ChartData integration with RangeSeries requires the series to implement
            // the trait - this will be connected when wire protocol integration is complete
            self.last_viewport_hash = viewport_hash;
        }

        self.needs_render = true;
    }

    fn render(
        &self,
        context: &mut RenderContext,
        _render_pass: &mut wgpu::RenderPass,
    ) -> Result<()> {
        if !self.cached_bars.is_empty() {
            let theme = context.theme().clone();
            self.render_cached_geometry(context, &theme)?;
        }
        Ok(())
    }

    fn needs_render(&self) -> bool {
        self.needs_render
    }

    fn z_order(&self) -> i32 {
        1
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
pub type CandlestickLayer = RangeBarLayer;
pub type CandlestickConfig = RangeBarConfig;
