//! Region shading layer
//!
//! Draws subtle vertical rectangles to visually differentiate time regions.
//! This is a generic layer - regions are configurable and have no domain knowledge.
//!
//! Use cases:
//! - Trading sessions (regular vs extended hours)
//! - Work shifts (day vs night)
//! - Peak vs off-peak periods
//! - Any time-based categorization

use crate::data::ChartData;
use crate::error::Result;
use crate::layers::{Layer, LayerStage};
use crate::renderer::RenderContext;
use crate::style::ChartStyle;
use crate::theme::{ChartTheme, Color};
use crate::viewport::Viewport;
use chrono::{DateTime, Timelike, Utc};

/// Defines how a region boundary is specified
#[derive(Debug, Clone, Copy)]
pub enum RegionBoundary {
    /// Seconds after midnight (0-86399), repeats daily
    DailySeconds(u32),
    /// Absolute Unix timestamp
    Timestamp(i64),
}

/// A shading region configuration
#[derive(Debug, Clone)]
pub struct ShadingRegion {
    /// Start of the region
    pub start: RegionBoundary,
    /// End of the region
    pub end: RegionBoundary,
    /// Opacity of the shading (0.0-1.0)
    pub opacity: f32,
    /// Optional label for the region
    pub label: Option<String>,
}

impl ShadingRegion {
    /// Create a daily repeating region
    pub fn daily(start_seconds: u32, end_seconds: u32, opacity: f32) -> Self {
        Self {
            start: RegionBoundary::DailySeconds(start_seconds),
            end: RegionBoundary::DailySeconds(end_seconds),
            opacity,
            label: None,
        }
    }

    /// Create a region with absolute timestamps
    pub fn absolute(start_ts: i64, end_ts: i64, opacity: f32) -> Self {
        Self {
            start: RegionBoundary::Timestamp(start_ts),
            end: RegionBoundary::Timestamp(end_ts),
            opacity,
            label: None,
        }
    }

    /// Add a label to the region
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

/// Configuration for region shading
#[derive(Debug, Clone)]
pub struct RegionShadingConfig {
    /// Regions to shade (outside these regions uses default_opacity)
    pub regions: Vec<ShadingRegion>,
    /// Opacity for areas outside defined regions
    pub default_opacity: f32,
}

impl Default for RegionShadingConfig {
    fn default() -> Self {
        // By default, no specific regions - everything is default opacity
        Self {
            regions: Vec::new(),
            default_opacity: 0.0,
        }
    }
}

impl RegionShadingConfig {
    /// Create config with a primary region (everything else is shaded)
    pub fn with_primary_region(
        start_seconds: u32,
        end_seconds: u32,
        primary_opacity: f32,
        other_opacity: f32,
    ) -> Self {
        Self {
            regions: vec![ShadingRegion::daily(
                start_seconds,
                end_seconds,
                primary_opacity,
            )],
            default_opacity: other_opacity,
        }
    }
}

/// Layer implementation
pub struct RegionShadingLayer {
    enabled: bool,
    cached_geometry_hash: u64,
    config: RegionShadingConfig,
    base_color: Color,
}

impl RegionShadingLayer {
    pub fn new() -> Self {
        Self {
            enabled: true,
            cached_geometry_hash: 0,
            config: RegionShadingConfig::default(),
            base_color: Color::rgba(0, 0, 0, 0),
        }
    }

    pub fn with_config(config: RegionShadingConfig) -> Self {
        let mut layer = Self::new();
        layer.config = config;
        layer
    }

    /// Set the regions to shade
    pub fn set_regions(&mut self, regions: Vec<ShadingRegion>) {
        self.config.regions = regions;
    }

    /// Set the default opacity for areas outside regions
    pub fn set_default_opacity(&mut self, opacity: f32) {
        self.config.default_opacity = opacity;
    }

    fn recompute_colors(&mut self, theme: &ChartTheme) {
        // Use theme overlay colour as base
        self.base_color = theme.colors.overlay;
    }

    fn geometry_hash(viewport: &Viewport) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = ahash::AHasher::default();
        viewport.visible_time_range().hash(&mut h);
        h.finish()
    }

    /// Get opacity for a given timestamp based on configured regions
    fn get_opacity_for_time(&self, secs_since_midnight: u32) -> f32 {
        for region in &self.config.regions {
            match (&region.start, &region.end) {
                (RegionBoundary::DailySeconds(start), RegionBoundary::DailySeconds(end)) => {
                    if *start <= *end {
                        // Normal case: start < end (e.g., 09:00-17:00)
                        if secs_since_midnight >= *start && secs_since_midnight < *end {
                            return region.opacity;
                        }
                    } else {
                        // Overnight case: start > end (e.g., 22:00-06:00)
                        if secs_since_midnight >= *start || secs_since_midnight < *end {
                            return region.opacity;
                        }
                    }
                }
                _ => {
                    // Absolute timestamps handled separately
                }
            }
        }
        self.config.default_opacity
    }
}

impl Default for RegionShadingLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Layer for RegionShadingLayer {
    fn name(&self) -> &str {
        "RegionShading"
    }

    fn stage(&self) -> LayerStage {
        LayerStage::ChartBackground
    }

    fn update(
        &mut self,
        _data: &ChartData,
        viewport: &Viewport,
        theme: &ChartTheme,
        _style: &ChartStyle,
    ) {
        self.recompute_colors(theme);
        let hash = Self::geometry_hash(viewport);
        if hash != self.cached_geometry_hash {
            self.cached_geometry_hash = hash;
        }
    }

    fn render(&self, ctx: &mut RenderContext, _render_pass: &mut wgpu::RenderPass) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        // If no regions configured and default opacity is 0, nothing to render
        if self.config.regions.is_empty() && self.config.default_opacity <= 0.0 {
            return Ok(());
        }

        let viewport = ctx.viewport();
        let (t_start, t_end) = viewport.visible_time_range();
        let mut t = t_start;
        let secs_per_day = 86_400_i64;

        while t < t_end {
            let dt =
                DateTime::<Utc>::from_timestamp(t, 0).expect("timestamp within DateTime range");
            let secs_since_midnight = dt.num_seconds_from_midnight();

            let opacity = self.get_opacity_for_time(secs_since_midnight);
            let color = self.base_color.with_alpha(opacity);

            // Determine segment end (find next boundary)
            let mut next_boundary = secs_per_day;
            for region in &self.config.regions {
                if let (RegionBoundary::DailySeconds(start), RegionBoundary::DailySeconds(end)) =
                    (&region.start, &region.end)
                {
                    if *start > secs_since_midnight && (*start as i64) < next_boundary {
                        next_boundary = *start as i64;
                    }
                    if *end > secs_since_midnight && (*end as i64) < next_boundary {
                        next_boundary = *end as i64;
                    }
                }
            }

            let day_start = dt
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .expect("valid midnight for date")
                .and_utc();
            let segment_end_ts = (day_start.timestamp() + next_boundary).min(t_end);

            // Draw rectangle if opacity > 0
            if opacity > 0.001 {
                let vp = ctx.viewport();
                let chart_rect = vp.chart_content_rect();
                let x0 = vp.chart_to_screen_x(t as f32);
                let x1 = vp.chart_to_screen_x(segment_end_ts as f32);
                let width = (x1 - x0).abs();
                if width > 0.5 {
                    let rect = crate::viewport::Rect::new(
                        x0.min(x1),
                        chart_rect.y,
                        width,
                        chart_rect.height,
                    );
                    ctx.draw_rect(rect, color);
                }
            }

            t = segment_end_ts;
        }
        Ok(())
    }

    fn needs_render(&self) -> bool {
        true
    }

    fn z_order(&self) -> i32 {
        -5 // Behind grid/data but above background
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

// Type alias for backwards compatibility
pub type SessionShadingLayer = RegionShadingLayer;
pub type SessionShadingConfig = RegionShadingConfig;
