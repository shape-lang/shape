//! Data row access methods for ExecutionContext
//!
//! Handles retrieving data rows by index, range, and timeframe.

use crate::data::OwnedDataRow as RowValue;
use crate::data::Timeframe;
use shape_ast::error::{Result, ShapeError};

impl super::ExecutionContext {
    /// Set the current data row index
    pub fn set_current_row(&mut self, index: usize) -> Result<()> {
        // With DuckDB, we don't track current row index the same way
        self.current_row_index = index;
        Ok(())
    }

    /// Get the current data row index
    pub fn current_row_index(&self) -> usize {
        self.current_row_index
    }

    /// Get the current data row index (alias for compatibility)
    pub fn get_current_row_index(&self) -> usize {
        self.current_row_index
    }

    /// Get total row count
    pub fn row_count(&self) -> usize {
        if let Some(ref cache) = self.data_cache {
            if let (Ok(id), Ok(timeframe)) = (self.get_current_id(), self.get_current_timeframe()) {
                return cache.row_count(&id, &timeframe);
            }
        }
        0
    }

    /// Get timestamp for a row index
    pub fn get_row_timestamp(&self, index: usize) -> Result<i64> {
        if let Some(ref cache) = self.data_cache {
            let id = self.get_current_id()?;
            let timeframe = self.get_current_timeframe()?;

            if let Some(row) = cache.get_row(&id, &timeframe, index) {
                return Ok(row.timestamp);
            }
        }

        Err(ShapeError::DataError {
            message: format!("No timestamp available at index {}", index),
            symbol: self.current_id.clone(),
            timeframe: self.current_timeframe.as_ref().map(|t| t.to_string()),
        })
    }

    /// Get a data row by relative index
    pub fn get_row(&mut self, relative_index: i32) -> Result<RowValue> {
        tracing::trace!("get_row called with relative_index: {}", relative_index);

        // Check look-ahead bias
        if let Some(ref guard) = self.lookahead_guard {
            guard.check_row_index(relative_index, "get_row")?;
        }

        // Phase 6: Check data_cache first (sync access to prefetched data)
        if let Some(ref cache) = self.data_cache {
            let id = self.get_current_id()?;
            let timeframe = self.get_current_timeframe()?;
            let absolute_index = (self.current_row_index as i32 + relative_index) as usize;

            if let Some(row) = cache.get_row(&id, &timeframe, absolute_index) {
                return Ok(row);
            }
        }

        // No data available - this is an error
        // With the async architecture, data must be prefetched before execution
        Err(ShapeError::DataError {
            message: format!(
                "No data available at index {}. Data must be prefetched before execution.",
                relative_index
            ),
            symbol: self.current_id.clone(), // Field name in ShapeError is still 'symbol' for compatibility
            timeframe: self.current_timeframe.as_ref().map(|tf| tf.to_string()),
        })
    }

    /// Get data rows in a range
    pub fn get_row_range(&mut self, start: i32, end: i32) -> Result<Vec<RowValue>> {
        // Check look-ahead bias for range access
        if let Some(ref guard) = self.lookahead_guard {
            // Check both start and end indices
            guard.check_row_index(start, "get_row_range_start")?;
            guard.check_row_index(end, "get_row_range_end")?;
        }

        if start > end {
            return Err(ShapeError::RuntimeError {
                message: format!("Invalid row range: {} to {}", start, end),
                location: None,
            });
        }

        // Phase 6: Check data_cache first (sync access to prefetched data)
        if let Some(ref cache) = self.data_cache {
            let id = self.get_current_id()?;
            let timeframe = self.get_current_timeframe()?;

            let mut rows = Vec::new();
            for i in start..=end {
                let absolute_index = (self.current_row_index as i32 + i) as usize;
                if let Some(row) = cache.get_row(&id, &timeframe, absolute_index) {
                    rows.push(row);
                }
            }
            return Ok(rows);
        }

        Ok(Vec::new())
    }

    /// Get a data row with explicit timeframe
    pub fn get_row_with_timeframe(
        &mut self,
        index: i32,
        timeframe: &Timeframe,
    ) -> Result<RowValue> {
        // Phase 6: Check data_cache first
        if let Some(ref cache) = self.data_cache {
            let id = self.get_current_id()?;
            let absolute_index = (self.current_row_index as i32 + index) as usize;

            if let Some(row) = cache.get_row(&id, timeframe, absolute_index) {
                return Ok(row);
            }
        }

        Err(ShapeError::DataError {
            message: format!(
                "No data available for timeframe {} at index {}",
                timeframe, index
            ),
            symbol: self.current_id.clone(),
            timeframe: Some(timeframe.to_string()),
        })
    }

    /// Get data rows in a range with explicit timeframe
    pub fn get_row_range_with_timeframe(
        &mut self,
        start: i32,
        end: i32,
        timeframe: &Timeframe,
    ) -> Result<Vec<RowValue>> {
        if start > end {
            return Err(ShapeError::RuntimeError {
                message: format!("Invalid row range: {} to {}", start, end),
                location: None,
            });
        }

        if let Some(ref cache) = self.data_cache {
            let id = self.get_current_id()?;

            let mut rows = Vec::new();
            for i in start..=end {
                let absolute_index = (self.current_row_index as i32 + i) as usize;
                if let Some(row) = cache.get_row(&id, timeframe, absolute_index) {
                    rows.push(row);
                }
            }
            return Ok(rows);
        }

        Ok(Vec::new())
    }

    /// Get a data row relative to the reference datetime
    pub fn get_row_relative_to_reference(&mut self, relative_index: i32) -> Result<RowValue> {
        // For now, this is equivalent to get_row since we don't have full DuckDB integration here
        self.get_row(relative_index)
    }

    /// Get data rows in a range relative to the reference datetime
    pub fn get_row_range_relative_to_reference(
        &mut self,
        start: i32,
        end: i32,
    ) -> Result<Vec<RowValue>> {
        self.get_row_range(start, end)
    }

    /// Get all rows for the current identifier and timeframe
    pub fn get_all_rows(&mut self) -> Result<Vec<RowValue>> {
        if let Some(ref cache) = self.data_cache {
            let id = self.get_current_id()?;
            let timeframe = self.get_current_timeframe()?;

            let count = cache.row_count(&id, &timeframe);
            let mut rows = Vec::with_capacity(count);
            for i in 0..count {
                if let Some(row) = cache.get_row(&id, &timeframe, i) {
                    rows.push(row);
                }
            }
            return Ok(rows);
        }

        Ok(Vec::new())
    }
}
