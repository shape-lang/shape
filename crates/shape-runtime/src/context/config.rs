//! Configuration methods for ExecutionContext
//!
//! Handles ID, timeframe, date range, and execution mode settings.

use super::super::lookahead_guard::{DataAccessMode, LookAheadGuard};
use crate::data::Timeframe;
use chrono::{DateTime, Utc};
use shape_ast::error::{Result, ShapeError};

impl super::ExecutionContext {
    /// Set the current ID
    pub fn set_id(&mut self, id: &str) {
        self.current_id = Some(id.to_string());
    }

    /// Get the ID being analyzed
    pub fn id(&self) -> Result<&str> {
        self.current_id
            .as_deref()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "No ID set".to_string(),
                location: None,
            })
    }

    /// Get the current ID (owned string)
    pub fn get_current_id(&self) -> Result<String> {
        self.current_id
            .clone()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "No data loaded".to_string(),
                location: None,
            })
    }

    /// Get the timeframe
    pub fn timeframe(&self) -> Result<&Timeframe> {
        self.current_timeframe
            .as_ref()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "No timeframe set".to_string(),
                location: None,
            })
    }

    /// Get the current timeframe
    pub fn get_current_timeframe(&self) -> Result<Timeframe> {
        self.current_timeframe
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "No timeframe set in context".to_string(),
                location: None,
            })
    }

    /// Set the current timeframe
    pub fn set_current_timeframe(&mut self, timeframe: Timeframe) -> Result<()> {
        self.current_timeframe = Some(timeframe);
        Ok(())
    }

    /// Update ID and timeframe from a DataFrame
    pub fn update_data(&mut self, data: &super::super::data::DataFrame) {
        self.current_id = Some(data.id.clone());
        self.current_timeframe = Some(data.timeframe);
        // Set current row to last row so [-1] gives most recent value
        // This is correct for non-simulation contexts where we want full history access
        self.current_row_index = if data.row_count() == 0 {
            0
        } else {
            data.row_count() - 1
        };
    }

    /// Get the loaded date range (if any) as native DateTime values
    pub fn get_date_range(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        self.date_range
    }

    /// Set the date range for data loading with native DateTime values
    pub fn set_date_range(&mut self, start: DateTime<Utc>, end: DateTime<Utc>) {
        self.date_range = Some((start, end));
    }

    /// Set the date range by parsing ISO8601 strings
    ///
    /// Accepts common date formats:
    /// - ISO8601: "2024-01-01T00:00:00Z"
    /// - Date only: "2024-01-01" (assumes start of day UTC)
    pub fn set_date_range_parsed(&mut self, start: &str, end: &str) -> Result<()> {
        let parse_date = |s: &str| -> Result<DateTime<Utc>> {
            // Try full ISO8601 first
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Ok(dt.with_timezone(&Utc));
            }
            // Try date-only format
            if let Ok(nd) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                return Ok(nd
                    .and_hms_opt(0, 0, 0)
                    .ok_or_else(|| ShapeError::RuntimeError {
                        message: format!("Invalid time for date: {}", s),
                        location: None,
                    })?
                    .and_utc());
            }
            Err(ShapeError::RuntimeError {
                message: format!("Cannot parse date '{}': expected ISO8601 or YYYY-MM-DD", s),
                location: None,
            })
        };

        let start_dt = parse_date(start)?;
        let end_dt = parse_date(end)?;
        self.date_range = Some((start_dt, end_dt));
        Ok(())
    }

    /// Set the reference datetime for row indexing
    pub fn set_reference_datetime(&mut self, datetime: DateTime<Utc>) {
        self.reference_datetime = Some(datetime);
    }

    /// Get the reference datetime
    pub fn get_reference_datetime(&self) -> Option<DateTime<Utc>> {
        self.reference_datetime
    }

    /// Update current row to match reference datetime
    pub fn sync_to_reference_datetime(&mut self) -> Result<()> {
        if self.reference_datetime.is_some() {
            self.current_row_index = 0; // Reset to beginning
        }
        Ok(())
    }

    /// Set the data access mode for future data validation
    pub fn set_data_access_mode(&mut self, mode: DataAccessMode, strict: bool) {
        self.lookahead_guard = Some(LookAheadGuard::new(mode, strict));
    }

    /// Get the look-ahead guard
    pub fn lookahead_guard(&self) -> Option<&LookAheadGuard> {
        self.lookahead_guard.as_ref()
    }
}
