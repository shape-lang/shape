//! Chart styling parameters to tune layout and layer appearance.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutStyle {
    pub price_axis_width: f32,
    pub time_axis_height: f32,
    pub chart_padding_x: f32,
    pub chart_padding_y: f32,
    pub volume_height_ratio: f32,
    pub volume_gap: f32,
}

impl Default for LayoutStyle {
    fn default() -> Self {
        Self {
            price_axis_width: 82.0,
            time_axis_height: 48.0,
            chart_padding_x: 12.0,
            chart_padding_y: 14.0,
            volume_height_ratio: 0.24,
            volume_gap: 4.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandleStyle {
    pub body_width_factor: f32,
    pub wick_width: f32,
    pub min_body_width: f32,
    pub max_body_width: f32,
    pub min_body_height: f32,
}

impl Default for CandleStyle {
    fn default() -> Self {
        Self {
            body_width_factor: 0.82,
            wick_width: 1.3,
            min_body_width: 3.0,
            max_body_width: 32.0,
            min_body_height: 1.2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeStyle {
    pub background_darken: f32,
    pub baseline_alpha: f32,
}

impl Default for VolumeStyle {
    fn default() -> Self {
        Self {
            background_darken: 0.22,
            baseline_alpha: 0.25,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentPriceStyle {
    pub line_width: f32,
    pub dash_length: f32,
    pub dash_gap: f32,
    pub label_padding: f32,
}

impl Default for CurrentPriceStyle {
    fn default() -> Self {
        Self {
            line_width: 1.2,
            dash_length: 8.0,
            dash_gap: 4.0,
            label_padding: 6.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChartStyle {
    pub layout: LayoutStyle,
    pub candles: CandleStyle,
    pub volume: VolumeStyle,
    pub current_price: CurrentPriceStyle,
}
