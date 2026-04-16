//! Generic load query for industry-agnostic data loading
//!
//! LoadQuery represents a data loading request with arbitrary parameters.
//! It's provider-agnostic - different providers interpret params differently.

use super::{DataQuery, Timeframe};
use shape_ast::error::{Result, ShapeError};
use shape_value::{ValueWord, ValueWordExt};
use std::collections::HashMap;

/// Generic data load request (industry-agnostic)
///
/// Supports arbitrary key-value parameters for maximum flexibility.
///
/// # Examples
///
/// Finance:
/// ```ignore
/// LoadQuery {
///     provider: Some("data"),
///     params: { "symbol": "ES", "from": "2023-01-01", "to": "2023-12-31" },
///     target_type: Some("Candle"),
/// }
/// ```
///
/// Weather:
/// ```ignore
/// LoadQuery {
///     provider: Some("weather_api"),
///     params: { "station": "LAX", "metric": "temperature", "interval": "hourly" },
///     target_type: Some("WeatherReading"),
/// }
/// ```
#[derive(Debug, Clone)]
pub struct LoadQuery {
    /// Provider name (e.g., "data", "api", "warehouse")
    /// If None, uses default provider
    pub provider: Option<String>,

    /// Generic parameters (arbitrary key-value)
    pub params: HashMap<String, ValueWord>,

    /// Target type name for validation (e.g., "Candle", "TickData")
    /// If specified, validates DataFrame has required columns
    pub target_type: Option<String>,

    /// Optional column mapping override
    /// Maps: target_field → source_column
    pub column_mapping: Option<HashMap<String, String>>,
}

impl LoadQuery {
    /// Create a new empty load query
    pub fn new() -> Self {
        Self {
            provider: None,
            params: HashMap::new(),
            target_type: None,
            column_mapping: None,
        }
    }

    /// Set provider name
    pub fn with_provider(mut self, name: &str) -> Self {
        self.provider = Some(name.to_string());
        self
    }

    /// Add a parameter
    pub fn with_param(mut self, key: &str, value: ValueWord) -> Self {
        self.params.insert(key.to_string(), value);
        self
    }

    /// Set target type for validation
    pub fn with_type(mut self, type_name: &str) -> Self {
        self.target_type = Some(type_name.to_string());
        self
    }

    /// Set column mapping
    pub fn with_column_mapping(mut self, mapping: HashMap<String, String>) -> Self {
        self.column_mapping = Some(mapping);
        self
    }

    /// Convert to provider-specific DataQuery
    ///
    /// Extracts common parameters and builds a DataQuery.
    /// Provider-specific logic for parameter interpretation.
    ///
    /// # Errors
    ///
    /// Returns error if required parameters are missing.
    pub fn to_data_query(&self) -> Result<DataQuery> {
        // Extract symbol (required)
        let symbol = self
            .params
            .get("symbol")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "data query requires 'symbol' parameter".to_string(),
                location: None,
            })?;

        // Extract timeframe (optional, defaults to 1m)
        let timeframe = self
            .params
            .get("timeframe")
            .and_then(|v| {
                if let Some(tf) = v.as_timeframe() {
                    return Some(*tf);
                }
                if let Some(duration) = v.as_duration() {
                    // Convert duration to timeframe
                    let value = duration.value;
                    let unit = duration.unit.clone();
                    use crate::data::TimeframeUnit;
                    use shape_ast::ast::DurationUnit;

                    if value <= 0.0 || value.fract() != 0.0 {
                        return None;
                    }

                    let tf_value = value as u32;
                    let tf_unit = match unit {
                        DurationUnit::Seconds => TimeframeUnit::Second,
                        DurationUnit::Minutes => TimeframeUnit::Minute,
                        DurationUnit::Hours => TimeframeUnit::Hour,
                        DurationUnit::Days => TimeframeUnit::Day,
                        DurationUnit::Weeks => TimeframeUnit::Week,
                        DurationUnit::Months => TimeframeUnit::Month,
                        _ => return None,
                    };

                    return Some(Timeframe::new(tf_value, tf_unit));
                }
                None
            })
            .unwrap_or(Timeframe::m1());

        let mut query = DataQuery::new(&symbol, timeframe);

        // Extract date range (from/to or start/end)
        let start_ts = self
            .params
            .get("from")
            .or_else(|| self.params.get("start"))
            .and_then(|v| self.value_to_timestamp(v));

        let end_ts = self
            .params
            .get("to")
            .or_else(|| self.params.get("end"))
            .and_then(|v| self.value_to_timestamp(v));

        if let (Some(start), Some(end)) = (start_ts, end_ts) {
            query = query.range(start, end);
        }

        // Extract limit
        if let Some(limit) = self
            .params
            .get("limit")
            .and_then(|v| v.as_f64().filter(|n| *n > 0.0).map(|n| n as usize))
        {
            query = query.limit(limit);
        }

        Ok(query)
    }

    /// Helper to convert ValueWord value to Unix timestamp
    fn value_to_timestamp(&self, value: &ValueWord) -> Option<i64> {
        if let Some(dt) = value.as_time() {
            return Some(dt.timestamp());
        }
        if let Some(s) = value.as_str() {
            // Parse date string "YYYY-MM-DD"
            use chrono::{DateTime, NaiveDate, Utc};

            if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                if let Some(dt) = date.and_hms_opt(0, 0, 0) {
                    let utc_dt = DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc);
                    return Some(utc_dt.timestamp());
                }
            }
            return None;
        }
        if let Some(n) = value.as_f64() {
            return Some(n as i64); // Assume Unix timestamp
        }
        None
    }
}

impl Default for LoadQuery {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::TimeframeUnit;
    use std::sync::Arc;

    #[test]
    fn test_basic_query() {
        let query = LoadQuery::new()
            .with_provider("data")
            .with_param("symbol", ValueWord::from_string(Arc::new("ES".to_string())))
            .with_type("Candle");

        assert_eq!(query.provider, Some("data".to_string()));
        assert_eq!(query.target_type, Some("Candle".to_string()));
        assert!(query.params.contains_key("symbol"));
    }

    #[test]
    fn test_to_data_query() {
        let query = LoadQuery::new()
            .with_param(
                "symbol",
                ValueWord::from_string(Arc::new("AAPL".to_string())),
            )
            .with_param(
                "timeframe",
                ValueWord::from_timeframe(Timeframe::new(5, TimeframeUnit::Minute)),
            );

        let data_query = query.to_data_query().unwrap();
        assert_eq!(data_query.id, "AAPL");
        assert_eq!(
            data_query.timeframe,
            Timeframe::new(5, TimeframeUnit::Minute)
        );
    }

    #[test]
    fn test_to_data_query_with_range() {
        use chrono::{DateTime, Utc};

        let start_ts = DateTime::parse_from_rfc3339("2023-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let end_ts = DateTime::parse_from_rfc3339("2023-12-31T23:59:59Z")
            .unwrap()
            .with_timezone(&Utc);

        let query = LoadQuery::new()
            .with_param("symbol", ValueWord::from_string(Arc::new("ES".to_string())))
            .with_param("from", ValueWord::from_time_utc(start_ts))
            .with_param("to", ValueWord::from_time_utc(end_ts));

        let data_query = query.to_data_query().unwrap();
        assert_eq!(data_query.id, "ES");
        assert!(data_query.start.is_some());
        assert!(data_query.end.is_some());
    }

    #[test]
    fn test_to_data_query_date_strings() {
        let query = LoadQuery::new()
            .with_param("symbol", ValueWord::from_string(Arc::new("ES".to_string())))
            .with_param(
                "from",
                ValueWord::from_string(Arc::new("2023-01-01".to_string())),
            )
            .with_param(
                "to",
                ValueWord::from_string(Arc::new("2023-12-31".to_string())),
            );

        let data_query = query.to_data_query().unwrap();
        assert_eq!(data_query.id, "ES");
        assert!(data_query.start.is_some());
        assert!(data_query.end.is_some());
    }

    #[test]
    fn test_to_data_query_missing_symbol() {
        let query = LoadQuery::new();
        assert!(query.to_data_query().is_err());
    }

    #[test]
    fn test_to_data_query_with_limit() {
        let query = LoadQuery::new()
            .with_param(
                "symbol",
                ValueWord::from_string(Arc::new("AAPL".to_string())),
            )
            .with_param("limit", ValueWord::from_f64(100.0));

        let data_query = query.to_data_query().unwrap();
        assert_eq!(data_query.limit, Some(100));
    }
}
