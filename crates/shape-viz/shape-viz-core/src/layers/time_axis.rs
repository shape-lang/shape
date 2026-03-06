//! Time axis rendering layer

use crate::data::ChartData;
use crate::error::Result;
use crate::layers::{Layer, LayerStage};
use crate::renderer::RenderContext;
use crate::style::ChartStyle;
use crate::theme::ChartTheme;
use crate::viewport::Viewport;
use chrono::{DateTime, Utc};

/// Layer for rendering time axis
#[derive(Debug)]
pub struct TimeAxisLayer {
    enabled: bool,
    needs_render: bool,
    axis_height: f32,
    _tick_length: f32,
    _label_offset: f32,
    show_grid_lines: bool,
}

impl TimeAxisLayer {
    pub fn new() -> Self {
        Self {
            enabled: true,
            needs_render: true,
            axis_height: 25.0, // Reduced from 30.0
            _tick_length: 4.0,
            _label_offset: 2.0, // Reduced from 3.0
            show_grid_lines: false,
        }
    }

    /// Set the height of the time axis
    pub fn set_axis_height(&mut self, height: f32) {
        if (self.axis_height - height).abs() > 0.1 {
            self.axis_height = height;
            self.needs_render = true;
        }
    }

    /// Find a nice time interval in seconds
    fn find_nice_time_interval(&self, seconds: f64) -> f64 {
        // Common time intervals in seconds
        let intervals = [
            1.0,       // 1 second
            5.0,       // 5 seconds
            10.0,      // 10 seconds
            30.0,      // 30 seconds
            60.0,      // 1 minute
            300.0,     // 5 minutes
            600.0,     // 10 minutes
            900.0,     // 15 minutes
            1800.0,    // 30 minutes
            3600.0,    // 1 hour
            7200.0,    // 2 hours
            14400.0,   // 4 hours
            21600.0,   // 6 hours
            43200.0,   // 12 hours
            86400.0,   // 1 day
            604800.0,  // 1 week
            2629746.0, // 1 month (approximate)
        ];

        // Find the closest interval
        intervals
            .iter()
            .min_by(|&&a, &&b| {
                let diff_a = (a - seconds).abs();
                let diff_b = (b - seconds).abs();
                diff_a
                    .partial_cmp(&diff_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
            .unwrap_or(seconds)
    }

    /// Format timestamp based on the interval
    fn format_time(&self, timestamp: i64, prev_timestamp: Option<i64>, interval: f64) -> String {
        let dt = DateTime::<Utc>::from_timestamp(timestamp, 0).unwrap();
        let prev_dt = prev_timestamp.map(|ts| DateTime::<Utc>::from_timestamp(ts, 0).unwrap());

        let show_date = match prev_dt {
            Some(prev) => dt.date_naive() != prev.date_naive(),
            None => true, // Always show date for the first label
        };

        if show_date {
            return dt.format("%d %b").to_string();
        }

        match interval {
            i if i < 60.0 => dt.format("%H:%M:%S").to_string(),
            _ => dt.format("%H:%M").to_string(),
        }
    }
}

impl Default for TimeAxisLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Layer for TimeAxisLayer {
    fn name(&self) -> &str {
        "TimeAxis"
    }

    fn stage(&self) -> LayerStage {
        LayerStage::TimeAxis
    }

    fn update(
        &mut self,
        _data: &ChartData,
        _viewport: &Viewport,
        _theme: &ChartTheme,
        _style: &ChartStyle,
    ) {
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
        let axis_rect = viewport.time_axis_rect();
        let chart_bounds = &viewport.chart_bounds;

        // The axis line is at the top edge of the axis area (boundary between content and axis)
        let _axis_line_y = content_rect.y + content_rect.height;

        // Draw axis background
        context.draw_rect(axis_rect, theme.colors.axis_background);

        // Don't draw axis line - reference chart has no visible axis lines

        // Calculate time range and steps
        let time_range_seconds = chart_bounds.time_duration().num_seconds() as f64;
        let target_label_count = (content_rect.width / 70.0) as i32; // Roughly 70 pixels between labels
        let raw_step = time_range_seconds / target_label_count as f64;
        let nice_step = self.find_nice_time_interval(raw_step);

        // Start from a nice round timestamp
        let start_timestamp =
            ((chart_bounds.time_start.timestamp() as f64 / nice_step).floor() * nice_step) as i64;
        let mut current_timestamp = start_timestamp;

        // Draw time labels and ticks
        let mut prev_timestamp = None;
        while current_timestamp <= chart_bounds.time_end.timestamp() {
            // Convert timestamp to screen coordinates
            let chart_pos = glam::Vec2::new(current_timestamp as f32, 0.0);
            let screen_pos = viewport.chart_to_screen(chart_pos);

            if screen_pos.x >= content_rect.x && screen_pos.x <= content_rect.x + content_rect.width
            {
                // Don't draw tick marks - reference chart has no visible ticks

                // Draw time label centered on the tick mark
                let time_text = self.format_time(current_timestamp, prev_timestamp, nice_step);

                #[cfg(feature = "text-rendering")]
                {
                    use crate::text::{TextAnchor, TextBaseline};
                    context.draw_text_anchored(
                        &time_text,
                        screen_pos.x,                         // Center on the grid line
                        axis_rect.y + axis_rect.height / 2.0, // Center vertically in axis
                        theme.colors.axis_label,
                        Some(theme.typography.secondary_font_size),
                        TextAnchor::Middle,
                        TextBaseline::Middle,
                    );
                }

                // Optionally draw grid line
                if self.show_grid_lines {
                    context.draw_line(
                        [screen_pos.x, content_rect.y],
                        [screen_pos.x, content_rect.y + content_rect.height],
                        theme.colors.grid_minor,
                        0.5,
                    );
                }
            }

            prev_timestamp = Some(current_timestamp);
            current_timestamp += nice_step as i64;
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
}
