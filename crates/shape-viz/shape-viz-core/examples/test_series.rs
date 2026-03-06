//! Simple test series for examples
//!
//! Provides generic test data without market_data dependency.

use shape_viz_core::data::{ChartData, RangeSeries, Series};
use std::any::Any;

/// Simple test series for examples
#[derive(Debug)]
pub struct TestRangeSeries {
    pub name: String,
    pub timestamps: Vec<i64>,
    pub opens: Vec<f64>,
    pub highs: Vec<f64>,
    pub lows: Vec<f64>,
    pub closes: Vec<f64>,
    pub volumes: Vec<f64>,
}

impl TestRangeSeries {
    /// Create test data with a sine wave pattern
    pub fn sine_wave(name: &str, count: usize, start_price: f64) -> Self {
        let start_ts = chrono::Utc::now().timestamp() - (count as i64 * 300);
        let mut timestamps = Vec::with_capacity(count);
        let mut opens = Vec::with_capacity(count);
        let mut highs = Vec::with_capacity(count);
        let mut lows = Vec::with_capacity(count);
        let mut closes = Vec::with_capacity(count);
        let mut volumes = Vec::with_capacity(count);

        let mut price = start_price;

        for i in 0..count {
            let open = price;
            let change = (i as f64 * 0.1).sin() * 2.0;
            price += change;
            let close = price;
            let high = open.max(close) + 0.3;
            let low = open.min(close) - 0.3;
            let volume = 1000.0 + (i as f64 * 0.05).sin() * 500.0;

            timestamps.push(start_ts + (i as i64 * 300)); // 5min intervals
            opens.push(open);
            highs.push(high);
            lows.push(low);
            closes.push(close);
            volumes.push(volume);
        }

        Self {
            name: name.to_string(),
            timestamps,
            opens,
            highs,
            lows,
            closes,
            volumes,
        }
    }

    pub fn into_chart_data(self) -> ChartData {
        ChartData::new(Box::new(self))
    }
}

impl Series for TestRangeSeries {
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
        self.closes[index]
    }

    fn get_x_range(&self) -> (f64, f64) {
        if self.timestamps.is_empty() {
            return (0.0, 0.0);
        }
        (
            self.timestamps[0] as f64,
            self.timestamps[self.len() - 1] as f64,
        )
    }

    fn get_y_range(&self, x_min: f64, x_max: f64) -> (f64, f64) {
        let mut min_price = f64::MAX;
        let mut max_price = f64::MIN;

        for i in 0..self.len() {
            let ts = self.timestamps[i] as f64;
            if ts >= x_min && ts <= x_max {
                min_price = min_price.min(self.lows[i]);
                max_price = max_price.max(self.highs[i]);
            }
        }

        if min_price == f64::MAX {
            (0.0, 0.0)
        } else {
            (min_price, max_price)
        }
    }

    fn find_index(&self, x: f64) -> Option<usize> {
        let target = x as i64;
        self.timestamps.binary_search(&target).ok().or_else(|| {
            let pos = self.timestamps.partition_point(|&ts| ts < target);
            if pos < self.len() {
                Some(pos)
            } else if !self.timestamps.is_empty() {
                Some(self.len() - 1)
            } else {
                None
            }
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl RangeSeries for TestRangeSeries {
    fn get_range(&self, index: usize) -> (f64, f64, f64, f64) {
        (
            self.opens[index],
            self.highs[index],
            self.lows[index],
            self.closes[index],
        )
    }

    fn get_auxiliary(&self, index: usize) -> Option<f64> {
        Some(self.volumes[index])
    }
}

/// Example demonstrating TestRangeSeries usage
#[allow(dead_code)]
fn main() {
    let series = TestRangeSeries::sine_wave("Test", 100, 100.0);
    println!(
        "Created test series '{}' with {} data points",
        series.name,
        series.len()
    );
    println!("X range: {:?}", series.get_x_range());
    let (y_min, y_max) = series.get_y_range(0.0, f64::MAX);
    println!("Y range: {:.2} - {:.2}", y_min, y_max);
}
