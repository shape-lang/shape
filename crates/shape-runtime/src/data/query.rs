//! Data query specification
//!
//! Describes what data to load from a DataProvider.

use super::Timeframe;

/// Query specification for loading data
#[derive(Debug, Clone)]
pub struct DataQuery {
    /// ID to query
    pub id: String,
    /// Timeframe of the data
    pub timeframe: Timeframe,
    /// Start timestamp (Unix seconds), inclusive
    pub start: Option<i64>,
    /// End timestamp (Unix seconds), inclusive
    pub end: Option<i64>,
    /// Maximum number of rows to return
    pub limit: Option<usize>,
}

impl DataQuery {
    /// Create a new query for an ID and timeframe
    pub fn new(id: &str, timeframe: Timeframe) -> Self {
        Self {
            id: id.to_string(),
            timeframe,
            start: None,
            end: None,
            limit: None,
        }
    }

    /// Set the time range (builder pattern)
    pub fn range(mut self, start: i64, end: i64) -> Self {
        self.start = Some(start);
        self.end = Some(end);
        self
    }

    /// Set the start time
    pub fn start(mut self, start: i64) -> Self {
        self.start = Some(start);
        self
    }

    /// Set the end time
    pub fn end(mut self, end: i64) -> Self {
        self.end = Some(end);
        self
    }

    /// Set the limit (builder pattern)
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    /// Check if this query has a time range
    pub fn has_range(&self) -> bool {
        self.start.is_some() || self.end.is_some()
    }
}

impl Default for DataQuery {
    fn default() -> Self {
        Self::new("", Timeframe::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_builder() {
        let query = DataQuery::new("AAPL", Timeframe::d1())
            .range(1000, 2000)
            .limit(100);

        assert_eq!(query.id, "AAPL");
        assert_eq!(query.start, Some(1000));
        assert_eq!(query.end, Some(2000));
        assert_eq!(query.limit, Some(100));
    }
}
