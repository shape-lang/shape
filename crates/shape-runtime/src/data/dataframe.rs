//! Generic DataFrame for columnar time series data
//!
//! Industry-agnostic storage for any time series data with named columns.

use super::Timeframe;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Generic columnar storage for time series data
///
/// Stores data as named columns of f64 values, plus timestamps.
/// No knowledge of specific column names (open, high, low, close, etc.)
/// is encoded here - that's determined by the data source.
#[derive(Debug, Clone)]
pub struct DataFrame {
    /// Column name -> column data
    pub columns: HashMap<String, Vec<f64>>,
    /// Timestamps (always present, Unix seconds)
    pub timestamps: Vec<i64>,
    /// Generic identifier
    pub id: String,
    /// Timeframe of the data
    pub timeframe: Timeframe,
}

impl DataFrame {
    /// Create a new empty DataFrame
    pub fn new(id: &str, timeframe: Timeframe) -> Self {
        Self {
            columns: HashMap::new(),
            timestamps: Vec::new(),
            id: id.to_string(),
            timeframe,
        }
    }

    /// Create a DataFrame with pre-allocated capacity
    pub fn with_capacity(id: &str, timeframe: Timeframe, capacity: usize) -> Self {
        Self {
            columns: HashMap::new(),
            timestamps: Vec::with_capacity(capacity),
            id: id.to_string(),
            timeframe,
        }
    }

    /// Create from a list of rows
    pub fn from_rows(id: &str, timeframe: Timeframe, rows: Vec<OwnedDataRow>) -> Self {
        if rows.is_empty() {
            return Self::new(id, timeframe);
        }

        let len = rows.len();
        let mut columns: HashMap<String, Vec<f64>> = HashMap::new();
        let mut timestamps = Vec::with_capacity(len);

        // Infer schema from all rows (to handle sparse data if any) or just first row?
        // Generic approach: iterate all rows
        for row in &rows {
            timestamps.push(row.timestamp);
            for (key, value) in &row.fields {
                columns
                    .entry(key.clone())
                    .or_insert_with(|| Vec::with_capacity(len))
                    .push(*value);
            }
        }

        // Pad shorter columns with NAN if necessary (though usually rows are uniform)
        for col in columns.values_mut() {
            while col.len() < timestamps.len() {
                col.push(f64::NAN);
            }
        }

        Self {
            columns,
            timestamps,
            id: id.to_string(),
            timeframe,
        }
    }

    /// Add a column of data
    pub fn add_column(&mut self, name: &str, data: Vec<f64>) {
        self.columns.insert(name.to_string(), data);
    }

    /// Get a column by name
    pub fn get_column(&self, name: &str) -> Option<&[f64]> {
        self.columns.get(name).map(|v| v.as_slice())
    }

    /// Get a mutable column by name
    pub fn get_column_mut(&mut self, name: &str) -> Option<&mut Vec<f64>> {
        self.columns.get_mut(name)
    }

    /// Get the number of rows
    pub fn row_count(&self) -> usize {
        self.timestamps.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.timestamps.is_empty()
    }

    /// Get the number of columns
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Get column names
    pub fn column_names(&self) -> impl Iterator<Item = &str> {
        self.columns.keys().map(|s| s.as_str())
    }

    /// Check if a column exists
    pub fn has_column(&self, name: &str) -> bool {
        self.columns.contains_key(name)
    }

    /// Get a row view at the given index
    pub fn get_row(&self, index: usize) -> Option<DataRow<'_>> {
        if index < self.row_count() {
            Some(DataRow {
                dataframe: self,
                index,
            })
        } else {
            None
        }
    }

    /// Get timestamp at index
    pub fn get_timestamp(&self, index: usize) -> Option<i64> {
        self.timestamps.get(index).copied()
    }

    /// Get value at (row, column)
    pub fn get_value(&self, row: usize, column: &str) -> Option<f64> {
        self.columns
            .get(column)
            .and_then(|col| col.get(row))
            .copied()
    }

    /// Create a schema from this DataFrame's columns
    pub fn schema(&self) -> Vec<String> {
        self.columns.keys().cloned().collect()
    }

    /// Slice the DataFrame to a range of rows
    pub fn slice(&self, start: usize, end: usize) -> Self {
        let end = end.min(self.row_count());
        let start = start.min(end);

        let mut df = Self::new(&self.id, self.timeframe);
        df.timestamps = self.timestamps[start..end].to_vec();

        for (name, col) in &self.columns {
            df.columns.insert(name.clone(), col[start..end].to_vec());
        }

        df
    }
}

impl Default for DataFrame {
    fn default() -> Self {
        Self::new("", Timeframe::default())
    }
}

/// A borrowed view of a single row in a DataFrame
///
/// Zero-copy access to row data - just stores reference and index.
#[derive(Debug, Clone, Copy)]
pub struct DataRow<'a> {
    dataframe: &'a DataFrame,
    index: usize,
}

/// An owned data row with generic fields
///
/// This struct provides an industry-agnostic type for storing arbitrary
/// data rows. It uses a HashMap for field storage to support any schema.
///
/// For performance-critical paths, the JIT compiler generates optimized
/// code when the type schema is known at compile time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnedDataRow {
    /// Unix timestamp (seconds) - always present in time series data
    pub timestamp: i64,
    /// Generic field storage - any f64 fields
    pub fields: std::collections::HashMap<String, f64>,
}

// DELETED: Legacy OHLCV accessor methods
// Use generic row.get_field("field_name") instead
// Finance-specific field names belong in stdlib, not Rust core

impl OwnedDataRow {
    /// Create a new generic OwnedDataRow with arbitrary fields
    pub fn new_generic(timestamp: i64, fields: std::collections::HashMap<String, f64>) -> Self {
        Self { timestamp, fields }
    }

    /// Create from HashMap of fields (alias for new_generic)
    pub fn from_hashmap(timestamp: i64, fields: std::collections::HashMap<String, f64>) -> Self {
        Self::new_generic(timestamp, fields)
    }

    /// Create from a DataRow by copying all available columns
    pub fn from_data_row(row: &DataRow<'_>) -> Option<Self> {
        let mut fields = std::collections::HashMap::new();

        // Copy all columns from the DataFrame
        for col_name in row.dataframe.columns.keys() {
            if let Some(value) = row.get(col_name) {
                fields.insert(col_name.clone(), value);
            }
        }

        Some(Self {
            timestamp: row.timestamp(),
            fields,
        })
    }

    /// Get a field by name
    pub fn get_field(&self, field: &str) -> Option<f64> {
        self.fields.get(field).copied()
    }

    /// Set a field value
    pub fn set_field(&mut self, field: &str, value: f64) {
        self.fields.insert(field.to_string(), value);
    }

    /// Check if field exists
    pub fn has_field(&self, field: &str) -> bool {
        self.fields.contains_key(field)
    }

    /// Get all field names
    pub fn field_names(&self) -> impl Iterator<Item = &String> {
        self.fields.keys()
    }

    /// Get timestamp as DateTime<Utc>
    pub fn datetime(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(self.timestamp, 0).unwrap_or_else(chrono::Utc::now)
    }
}

impl<'a> DataRow<'a> {
    /// Get the timestamp for this row
    pub fn timestamp(&self) -> i64 {
        self.dataframe.timestamps[self.index]
    }

    /// Get the row index
    pub fn index(&self) -> usize {
        self.index
    }

    /// Get a field value by name
    pub fn get(&self, field: &str) -> Option<f64> {
        self.dataframe
            .columns
            .get(field)
            .and_then(|col| col.get(self.index))
            .copied()
    }

    /// Get a field value with a default
    pub fn get_or(&self, field: &str, default: f64) -> f64 {
        self.get(field).unwrap_or(default)
    }

    /// Check if a field exists
    pub fn has_field(&self, field: &str) -> bool {
        self.dataframe.columns.contains_key(field)
    }

    /// Get all field names
    pub fn fields(&self) -> impl Iterator<Item = &str> {
        self.dataframe.column_names()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dataframe_basic() {
        let mut df = DataFrame::new("TEST", Timeframe::d1());

        df.timestamps = vec![1000, 2000, 3000];
        df.add_column("value", vec![1.0, 2.0, 3.0]);
        df.add_column("other", vec![10.0, 20.0, 30.0]);

        assert_eq!(df.row_count(), 3);
        assert_eq!(df.column_count(), 2);
        assert!(df.has_column("value"));
        assert!(!df.has_column("missing"));

        assert_eq!(df.get_value(1, "value"), Some(2.0));
        assert_eq!(df.get_value(1, "other"), Some(20.0));
    }

    #[test]
    fn test_datarow_access() {
        let mut df = DataFrame::new("TEST", Timeframe::d1());
        df.timestamps = vec![1000, 2000, 3000];
        df.add_column("price", vec![100.0, 101.0, 102.0]);

        let row = df.get_row(1).unwrap();
        assert_eq!(row.timestamp(), 2000);
        assert_eq!(row.get("price"), Some(101.0));
        assert_eq!(row.get_or("missing", 0.0), 0.0);
    }

    #[test]
    fn test_dataframe_slice() {
        let mut df = DataFrame::new("TEST", Timeframe::d1());
        df.timestamps = vec![1000, 2000, 3000, 4000, 5000];
        df.add_column("value", vec![1.0, 2.0, 3.0, 4.0, 5.0]);

        let sliced = df.slice(1, 4);
        assert_eq!(sliced.row_count(), 3);
        assert_eq!(sliced.timestamps, vec![2000, 3000, 4000]);
        assert_eq!(sliced.get_column("value"), Some(&[2.0, 3.0, 4.0][..]));
    }
}
