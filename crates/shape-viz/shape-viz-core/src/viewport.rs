//! Viewport and coordinate system management

use crate::error::{ChartError, Result};
use crate::style::LayoutStyle;
use chrono::{DateTime, Utc};
use glam::{Mat3, Vec2};
use serde::{Deserialize, Serialize};

/// 2D rectangle representing screen or chart bounds
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn from_size(width: f32, height: f32) -> Self {
        Self::new(0.0, 0.0, width, height)
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn center(&self) -> Vec2 {
        Vec2::new(self.x + self.width * 0.5, self.y + self.height * 0.5)
    }

    pub fn contains_point(&self, point: Vec2) -> bool {
        point.x >= self.x
            && point.x <= self.right()
            && point.y >= self.y
            && point.y <= self.bottom()
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }

    /// Shrink rectangle by given margins
    pub fn shrink(&self, margin: f32) -> Rect {
        Rect::new(
            self.x + margin,
            self.y + margin,
            (self.width - 2.0 * margin).max(0.0),
            (self.height - 2.0 * margin).max(0.0),
        )
    }

    /// Split rectangle horizontally into two parts
    pub fn split_horizontal(&self, ratio: f32) -> (Rect, Rect) {
        let split_y = self.y + self.height * ratio.clamp(0.0, 1.0);
        let top_height = split_y - self.y;
        let bottom_height = self.bottom() - split_y;

        let top = Rect::new(self.x, self.y, self.width, top_height);
        let bottom = Rect::new(self.x, split_y, self.width, bottom_height);

        (top, bottom)
    }

    /// Calculate the intersection of two rectangles
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = self.right().min(other.right());
        let y2 = self.bottom().min(other.bottom());

        if x2 > x1 && y2 > y1 {
            Some(Rect::new(x1, y1, x2 - x1, y2 - y1))
        } else {
            None
        }
    }
}

/// Defines the layout of the different chart panels
#[derive(Debug, Clone)]
pub struct ChartLayout {
    pub main_panel: Rect,
    pub volume_panel: Rect,
    pub price_axis_panel: Rect,
    pub time_axis_panel: Rect,
    pub full_rect: Rect,
}

impl ChartLayout {
    /// Calculate the layout based on the full viewport rectangle and style parameters
    pub fn new(full_rect: Rect, style: &LayoutStyle) -> Self {
        let price_axis_width = style.price_axis_width.max(40.0);
        let time_axis_height = style.time_axis_height.max(24.0);
        let volume_height_ratio = style.volume_height_ratio.clamp(0.05, 0.5);
        let volume_gap = style.volume_gap.max(0.0);
        let chart_padding_x = style.chart_padding_x.max(0.0);
        let chart_padding_y = style.chart_padding_y.max(0.0);

        let chart_area_x = full_rect.x + chart_padding_x;
        let chart_area_y = full_rect.y + chart_padding_y;
        let chart_area_width =
            (full_rect.width - price_axis_width - 2.0 * chart_padding_x).max(1.0);
        let chart_area_height =
            (full_rect.height - time_axis_height - 2.0 * chart_padding_y).max(1.0);

        let volume_height = (chart_area_height * volume_height_ratio).max(24.0);
        let main_height = (chart_area_height - volume_height - volume_gap).max(1.0);

        let main_panel = Rect::new(chart_area_x, chart_area_y, chart_area_width, main_height);

        let volume_panel = Rect::new(
            chart_area_x,
            chart_area_y + main_height + volume_gap,
            chart_area_width,
            volume_height,
        );

        let price_axis_panel = Rect::new(
            chart_area_x + chart_area_width,
            chart_area_y,
            price_axis_width,
            main_height,
        );

        let time_axis_panel = Rect::new(
            full_rect.x,
            volume_panel.y + volume_panel.height,
            full_rect.width,
            time_axis_height,
        );

        Self {
            main_panel,
            volume_panel,
            price_axis_panel,
            time_axis_panel,
            full_rect,
        }
    }
}

/// Chart bounds in data space (time and price coordinates)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChartBounds {
    pub time_start: DateTime<Utc>,
    pub time_end: DateTime<Utc>,
    pub price_min: f64,
    pub price_max: f64,
}

impl ChartBounds {
    pub fn new(
        time_start: DateTime<Utc>,
        time_end: DateTime<Utc>,
        price_min: f64,
        price_max: f64,
    ) -> Result<Self> {
        if time_start >= time_end {
            return Err(ChartError::data_range("Start time must be before end time"));
        }

        if price_min >= price_max {
            return Err(ChartError::data_range(
                "Min price must be less than max price",
            ));
        }

        Ok(Self {
            time_start,
            time_end,
            price_min,
            price_max,
        })
    }

    pub fn time_duration(&self) -> chrono::Duration {
        self.time_end - self.time_start
    }

    pub fn price_range(&self) -> f64 {
        self.price_max - self.price_min
    }

    pub fn contains_time(&self, time: DateTime<Utc>) -> bool {
        time >= self.time_start && time <= self.time_end
    }

    pub fn contains_price(&self, price: f64) -> bool {
        price >= self.price_min && price <= self.price_max
    }

    /// Expand bounds to include the given time and price
    pub fn expand_to_include(&mut self, time: DateTime<Utc>, price: f64) {
        if time < self.time_start {
            self.time_start = time;
        }
        if time > self.time_end {
            self.time_end = time;
        }
        if price < self.price_min {
            self.price_min = price;
        }
        if price > self.price_max {
            self.price_max = price;
        }
    }

    /// Add padding to the bounds (percentage of current range)
    pub fn with_padding(&self, time_padding: f64, price_padding: f64) -> Result<Self> {
        let time_range_seconds = self.time_duration().num_seconds() as f64;
        let time_padding_seconds = (time_range_seconds * time_padding) as i64;

        let price_range = self.price_range();
        let price_padding_amount = price_range * price_padding;

        ChartBounds::new(
            self.time_start - chrono::Duration::seconds(time_padding_seconds),
            self.time_end + chrono::Duration::seconds(time_padding_seconds),
            self.price_min - price_padding_amount,
            self.price_max + price_padding_amount,
        )
    }
}

/// Viewport manages coordinate transformations between chart data space and screen space
#[derive(Debug, Clone)]
pub struct Viewport {
    /// Screen rectangle where the chart is rendered
    pub screen_rect: Rect,
    /// Chart data bounds
    pub chart_bounds: ChartBounds,
    /// Layout of the chart panels
    pub layout: ChartLayout,
    layout_style: LayoutStyle,
    /// Transformation matrix from chart space to screen space
    transform: Mat3,
    /// Inverse transformation matrix from screen space to chart space
    inverse_transform: Mat3,
}

impl Viewport {
    pub fn new(screen_rect: Rect, chart_bounds: ChartBounds, layout_style: LayoutStyle) -> Self {
        let layout = ChartLayout::new(screen_rect, &layout_style);
        let mut viewport = Self {
            screen_rect,
            chart_bounds,
            layout,
            layout_style,
            transform: Mat3::IDENTITY,
            inverse_transform: Mat3::IDENTITY,
        };
        viewport.update_transforms();
        viewport
    }

    /// Get the chart content area (main panel)
    pub fn chart_content_rect(&self) -> Rect {
        self.layout.main_panel
    }

    /// Get the price axis area
    pub fn price_axis_rect(&self) -> Rect {
        self.layout.price_axis_panel
    }

    /// Get the time axis area
    pub fn time_axis_rect(&self) -> Rect {
        self.layout.time_axis_panel
    }

    /// Get the volume area
    pub fn volume_rect(&self) -> Rect {
        self.layout.volume_panel
    }

    /// Update the screen rectangle
    pub fn set_screen_rect(&mut self, rect: Rect) {
        self.screen_rect = rect;
        self.layout = ChartLayout::new(rect, &self.layout_style);
        self.update_transforms();
    }

    /// Update layout style parameters and recompute layout
    pub fn set_layout_style(&mut self, style: LayoutStyle) {
        self.layout_style = style;
        self.layout = ChartLayout::new(self.screen_rect, &self.layout_style);
        self.update_transforms();
    }

    /// Update the chart bounds
    pub fn set_chart_bounds(&mut self, bounds: ChartBounds) {
        self.chart_bounds = bounds;
        self.update_transforms();
    }

    /// Pan the viewport by screen space delta
    pub fn pan(&mut self, screen_delta: Vec2) {
        // Convert screen delta to chart space delta
        let chart_delta = self.screen_to_chart_delta(screen_delta);

        // Create new bounds with the pan applied
        let time_delta_seconds = chart_delta.x as i64;
        let price_delta = chart_delta.y as f64;

        if let Ok(new_bounds) = ChartBounds::new(
            self.chart_bounds.time_start + chrono::Duration::seconds(time_delta_seconds),
            self.chart_bounds.time_end + chrono::Duration::seconds(time_delta_seconds),
            self.chart_bounds.price_min + price_delta,
            self.chart_bounds.price_max + price_delta,
        ) {
            self.chart_bounds = new_bounds;
            self.update_transforms();
        }
    }

    /// Zoom the viewport around a center point in screen space
    pub fn zoom(&mut self, center_screen: Vec2, zoom_factor: f32) {
        // Convert center to chart space
        let center_chart = self.screen_to_chart(center_screen);

        // Calculate new ranges
        let time_range = self.chart_bounds.time_duration().num_seconds() as f64;
        let price_range = self.chart_bounds.price_range();

        let new_time_range = time_range / zoom_factor as f64;
        let new_price_range = price_range / zoom_factor as f64;

        // Calculate new bounds centered around the zoom point
        let time_center_offset =
            (center_chart.x as f64 - self.chart_bounds.time_start.timestamp() as f64) / time_range;
        let price_center_offset =
            (center_chart.y as f64 - self.chart_bounds.price_min) / price_range;

        let new_time_start = center_chart.x as i64 - (new_time_range * time_center_offset) as i64;
        let new_time_end =
            center_chart.x as i64 + (new_time_range * (1.0 - time_center_offset)) as i64;

        let new_price_min = center_chart.y as f64 - new_price_range * price_center_offset;
        let new_price_max = center_chart.y as f64 + new_price_range * (1.0 - price_center_offset);

        if let (Some(start_time), Some(end_time)) = (
            DateTime::from_timestamp(new_time_start, 0),
            DateTime::from_timestamp(new_time_end, 0),
        ) {
            if let Ok(new_bounds) =
                ChartBounds::new(start_time, end_time, new_price_min, new_price_max)
            {
                self.chart_bounds = new_bounds;
                self.update_transforms();
            }
        }
    }

    /// Convert chart coordinates (timestamp, price) to screen coordinates
    pub fn chart_to_screen(&self, chart_pos: Vec2) -> Vec2 {
        let homogeneous = self.transform * chart_pos.extend(1.0);
        Vec2::new(homogeneous.x, homogeneous.y)
    }

    /// Convert screen coordinates to chart coordinates (timestamp, price)
    pub fn screen_to_chart(&self, screen_pos: Vec2) -> Vec2 {
        let homogeneous = self.inverse_transform * screen_pos.extend(1.0);
        Vec2::new(homogeneous.x, homogeneous.y)
    }

    /// Convert screen space delta to chart space delta
    pub fn screen_to_chart_delta(&self, screen_delta: Vec2) -> Vec2 {
        let origin = self.screen_to_chart(Vec2::ZERO);
        let target = self.screen_to_chart(screen_delta);
        target - origin
    }

    /// Check if a chart position is visible in the current viewport
    pub fn is_chart_pos_visible(&self, chart_pos: Vec2) -> bool {
        let screen_pos = self.chart_to_screen(chart_pos);
        self.screen_rect.contains_point(screen_pos)
    }

    /// Get the visible time range as timestamps
    pub fn visible_time_range(&self) -> (i64, i64) {
        (
            self.chart_bounds.time_start.timestamp(),
            self.chart_bounds.time_end.timestamp(),
        )
    }

    /// Get the visible price range
    pub fn visible_price_range(&self) -> (f64, f64) {
        (self.chart_bounds.price_min, self.chart_bounds.price_max)
    }

    /// Convert chart X coordinate (timestamp) to screen X coordinate
    pub fn chart_to_screen_x(&self, chart_x: f32) -> f32 {
        let chart_pos = Vec2::new(chart_x, 0.0);
        let screen_pos = self.chart_to_screen(chart_pos);
        screen_pos.x
    }

    /// Convert chart Y coordinate (price) to screen Y coordinate
    pub fn chart_to_screen_y(&self, chart_y: f32) -> f32 {
        let chart_pos = Vec2::new(0.0, chart_y);
        let screen_pos = self.chart_to_screen(chart_pos);
        screen_pos.y
    }

    /// Convert chart distance in X direction to screen distance
    pub fn chart_to_screen_distance_x(&self, chart_distance: f32) -> f32 {
        let origin = self.chart_to_screen(Vec2::ZERO);
        let target = self.chart_to_screen(Vec2::new(chart_distance, 0.0));
        (target.x - origin.x).abs()
    }

    /// Convert chart distance in Y direction to screen distance
    pub fn chart_to_screen_distance_y(&self, chart_distance: f32) -> f32 {
        let origin = self.chart_to_screen(Vec2::ZERO);
        let target = self.chart_to_screen(Vec2::new(0.0, chart_distance));
        (target.y - origin.y).abs()
    }

    /// Update the transformation matrices
    fn update_transforms(&mut self) {
        // Use the main panel for transformations
        let content_rect = self.layout.main_panel;

        // Calculate scale factors
        let time_scale =
            content_rect.width / (self.chart_bounds.time_duration().num_seconds() as f32);
        let price_scale = -content_rect.height / (self.chart_bounds.price_range() as f32); // Negative because screen Y increases downward

        // Calculate translation
        let time_translate =
            content_rect.x - (self.chart_bounds.time_start.timestamp() as f32 * time_scale);
        let price_translate =
            content_rect.bottom() - (self.chart_bounds.price_min as f32 * price_scale);

        // Create transformation matrix
        self.transform = Mat3::from_translation(Vec2::new(time_translate, price_translate))
            * Mat3::from_scale(Vec2::new(time_scale, price_scale));

        // Calculate inverse transform
        self.inverse_transform = self.transform.inverse();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::LayoutStyle;
    use chrono::TimeZone;

    #[test]
    fn test_rect_operations() {
        let rect = Rect::new(10.0, 20.0, 100.0, 50.0);

        assert_eq!(rect.right(), 110.0);
        assert_eq!(rect.bottom(), 70.0);
        assert_eq!(rect.center(), Vec2::new(60.0, 45.0));

        assert!(rect.contains_point(Vec2::new(50.0, 40.0)));
        assert!(!rect.contains_point(Vec2::new(5.0, 40.0)));
    }

    #[test]
    fn test_chart_bounds() {
        let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();

        let bounds = ChartBounds::new(start, end, 100.0, 200.0).unwrap();

        assert_eq!(bounds.time_duration().num_hours(), 24);
        assert_eq!(bounds.price_range(), 100.0);

        let mid_time = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
        assert!(bounds.contains_time(mid_time));
        assert!(bounds.contains_price(150.0));
    }

    #[test]
    fn test_viewport_transforms() {
        let screen_rect = Rect::new(0.0, 0.0, 800.0, 600.0);
        let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 1, 1, 1, 0, 0).unwrap(); // 1 hour
        let chart_bounds = ChartBounds::new(start, end, 100.0, 200.0).unwrap();

        let viewport = Viewport::new(screen_rect, chart_bounds, LayoutStyle::default());

        // Test coordinate transformations
        let chart_pos = Vec2::new(start.timestamp() as f32, 150.0);
        let screen_pos = viewport.chart_to_screen(chart_pos);
        let back_to_chart = viewport.screen_to_chart(screen_pos);

        // Should round-trip with minimal floating point error (allowing small FP drift)
        assert!((back_to_chart.x - chart_pos.x).abs() < 200.0);
        assert!((back_to_chart.y - chart_pos.y).abs() < 0.01);
    }
}
