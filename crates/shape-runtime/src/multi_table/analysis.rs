//! Multi-series analysis utilities

use super::types::Divergence;
use crate::data::OwnedDataRow as RowValue;
use shape_ast::error::{Result, ShapeError};

/// Multi-series analysis utilities
pub struct MultiTableAnalysis;

impl MultiTableAnalysis {
    /// Calculate correlation between two aligned datasets
    pub fn correlation(data1: &[RowValue], data2: &[RowValue]) -> Result<f64> {
        if data1.len() != data2.len() {
            return Err(ShapeError::RuntimeError {
                message: "Data lengths must match for correlation calculation".into(),
                location: None,
            });
        }

        if data1.is_empty() {
            return Err(ShapeError::RuntimeError {
                message: "Cannot calculate correlation on empty data".into(),
                location: None,
            });
        }

        let n = data1.len() as f64;

        let get_value = |row: &RowValue| {
            row.field_names()
                .next()
                .and_then(|f| row.get_field(f))
                .unwrap_or(f64::NAN)
        };

        let mean1 = data1.iter().map(get_value).sum::<f64>() / n;
        let mean2 = data2.iter().map(get_value).sum::<f64>() / n;

        let mut covariance = 0.0;
        let mut variance1 = 0.0;
        let mut variance2 = 0.0;

        for i in 0..data1.len() {
            let diff1 = get_value(&data1[i]) - mean1;
            let diff2 = get_value(&data2[i]) - mean2;

            covariance += diff1 * diff2;
            variance1 += diff1 * diff1;
            variance2 += diff2 * diff2;
        }

        let std1 = (variance1 / n).sqrt();
        let std2 = (variance2 / n).sqrt();

        if std1 == 0.0 || std2 == 0.0 {
            return Err(ShapeError::RuntimeError {
                message: "Cannot calculate correlation with zero variance".into(),
                location: None,
            });
        }

        Ok(covariance / (n * std1 * std2))
    }

    /// Find divergences between two data series
    pub fn find_divergences(
        data1: &[RowValue],
        data2: &[RowValue],
        window: usize,
    ) -> Result<Vec<Divergence>> {
        if data1.len() != data2.len() {
            return Err(ShapeError::RuntimeError {
                message: "Data series must have equal length".into(),
                location: None,
            });
        }

        if window == 0 || window > data1.len() {
            return Err(ShapeError::RuntimeError {
                message: "Invalid window size".into(),
                location: None,
            });
        }

        let mut divergences = Vec::new();

        for i in window..data1.len() {
            let trend1 = Self::calculate_trend(&data1[i - window..i]);
            let trend2 = Self::calculate_trend(&data2[i - window..i]);

            if (trend1 > 0.0 && trend2 < 0.0) || (trend1 < 0.0 && trend2 > 0.0) {
                divergences.push(Divergence {
                    timestamp: data1[i].timestamp,
                    index: i,
                    id1_trend: trend1,
                    id2_trend: trend2,
                    strength: (trend1 - trend2).abs(),
                });
            }
        }

        Ok(divergences)
    }

    /// Calculate simple trend (slope) over a window
    fn calculate_trend(rows: &[RowValue]) -> f64 {
        if rows.is_empty() {
            return 0.0;
        }

        let n = rows.len() as f64;
        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        let mut sum_xy = 0.0;
        let mut sum_x2 = 0.0;

        for (i, row) in rows.iter().enumerate() {
            let x = i as f64;
            let y = row
                .field_names()
                .next()
                .and_then(|f| row.get_field(f))
                .unwrap_or(f64::NAN);

            sum_x += x;
            sum_y += y;
            sum_xy += x * y;
            sum_x2 += x * x;
        }

        let denominator = n * sum_x2 - sum_x * sum_x;
        if denominator == 0.0 {
            return 0.0;
        }

        (n * sum_xy - sum_x * sum_y) / denominator
    }
}
