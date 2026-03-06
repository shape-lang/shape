//! Generic chart data adapter
//!
//! Provides adapters for converting generic time-series data to viz-core's
//! Series and RangeSeries traits without depending on market_data crate.

use shape_viz_core::data::{ChartData, RangeSeries, Series};
use std::any::Any;

/// Generic OHLCV data structure (no market_data dependency)
#[derive(Debug, Clone)]
pub struct GenericOHLCV {
    pub symbol: String,
    pub timestamps: Vec<i64>,
    pub opens: Vec<f64>,
    pub highs: Vec<f64>,
    pub lows: Vec<f64>,
    pub closes: Vec<f64>,
    pub volumes: Vec<f64>,
}

impl GenericOHLCV {
    pub fn new(symbol: String) -> Self {
        Self {
            symbol,
            timestamps: Vec::new(),
            opens: Vec::new(),
            highs: Vec::new(),
            lows: Vec::new(),
            closes: Vec::new(),
            volumes: Vec::new(),
        }
    }

    pub fn with_capacity(symbol: String, capacity: usize) -> Self {
        Self {
            symbol,
            timestamps: Vec::with_capacity(capacity),
            opens: Vec::with_capacity(capacity),
            highs: Vec::with_capacity(capacity),
            lows: Vec::with_capacity(capacity),
            closes: Vec::with_capacity(capacity),
            volumes: Vec::with_capacity(capacity),
        }
    }

    pub fn push(
        &mut self,
        timestamp: i64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: f64,
    ) {
        self.timestamps.push(timestamp);
        self.opens.push(open);
        self.highs.push(high);
        self.lows.push(low);
        self.closes.push(close);
        self.volumes.push(volume);
    }

    pub fn len(&self) -> usize {
        self.timestamps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.timestamps.is_empty()
    }
}

/// Adapter that implements viz-core's Series and RangeSeries traits
#[derive(Debug)]
pub struct GenericSeriesAdapter {
    data: GenericOHLCV,
}

impl GenericSeriesAdapter {
    pub fn new(data: GenericOHLCV) -> Self {
        Self { data }
    }

    pub fn into_chart_data(self) -> ChartData {
        ChartData::new(Box::new(self))
    }
}

/// Create ChartData from GenericOHLCV (convenience function)
pub fn chart_data_from_ohlcv(data: GenericOHLCV) -> ChartData {
    GenericSeriesAdapter::new(data).into_chart_data()
}

impl Series for GenericSeriesAdapter {
    fn name(&self) -> &str {
        &self.data.symbol
    }

    fn len(&self) -> usize {
        self.data.len()
    }

    fn get_x(&self, index: usize) -> f64 {
        // Timestamps are in milliseconds, convert to seconds for viz
        (self.data.timestamps[index] / 1000) as f64
    }

    fn get_y(&self, index: usize) -> f64 {
        // Default Y value is close price
        self.data.closes[index]
    }

    fn get_x_range(&self) -> (f64, f64) {
        if self.data.is_empty() {
            return (0.0, 0.0);
        }
        let min_ts = (self.data.timestamps[0] / 1000) as f64;
        let max_ts = (self.data.timestamps[self.data.len() - 1] / 1000) as f64;
        (min_ts, max_ts)
    }

    fn get_y_range(&self, x_min: f64, x_max: f64) -> (f64, f64) {
        let mut min_price = f64::MAX;
        let mut max_price = f64::MIN;

        for i in 0..self.data.len() {
            let ts = (self.data.timestamps[i] / 1000) as f64;
            if ts >= x_min && ts <= x_max {
                min_price = min_price.min(self.data.lows[i]);
                max_price = max_price.max(self.data.highs[i]);
            }
        }

        if min_price == f64::MAX {
            (0.0, 0.0)
        } else {
            (min_price, max_price)
        }
    }

    fn find_index(&self, x: f64) -> Option<usize> {
        let target_ts = (x * 1000.0) as i64;
        self.data
            .timestamps
            .binary_search(&target_ts)
            .ok()
            .or_else(|| {
                // Find closest index
                let pos = self.data.timestamps.partition_point(|&ts| ts < target_ts);
                if pos < self.data.len() {
                    Some(pos)
                } else if !self.data.is_empty() {
                    Some(self.data.len() - 1)
                } else {
                    None
                }
            })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl RangeSeries for GenericSeriesAdapter {
    fn get_range(&self, index: usize) -> (f64, f64, f64, f64) {
        (
            self.data.opens[index],
            self.data.highs[index],
            self.data.lows[index],
            self.data.closes[index],
        )
    }

    fn get_auxiliary(&self, index: usize) -> Option<f64> {
        Some(self.data.volumes[index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generic_ohlcv() {
        let mut data = GenericOHLCV::new("TEST".to_string());
        data.push(1000, 100.0, 105.0, 95.0, 102.0, 1000.0);
        data.push(2000, 102.0, 108.0, 100.0, 106.0, 1500.0);

        assert_eq!(data.len(), 2);
        assert_eq!(data.symbol, "TEST");
    }

    #[test]
    fn test_series_adapter() {
        let mut data = GenericOHLCV::new("TEST".to_string());
        data.push(1000, 100.0, 105.0, 95.0, 102.0, 1000.0);

        let adapter = GenericSeriesAdapter::new(data);
        assert_eq!(adapter.name(), "TEST");
        assert_eq!(adapter.len(), 1);

        let (open, high, low, close) = adapter.get_range(0);
        assert_eq!(open, 100.0);
        assert_eq!(high, 105.0);
        assert_eq!(low, 95.0);
        assert_eq!(close, 102.0);

        assert_eq!(adapter.get_auxiliary(0), Some(1000.0));
    }
}
