//! Builder for constructing binary columnar data
//!
//! Plugins use this to build binary data that can be directly loaded into SeriesStorage.
//!
//! # Example
//!
//! ```ignore
//! use shape_abi_v1::binary_builder::BinaryDataBuilder;
//!
//! let data = BinaryDataBuilder::new()
//!     .add_timestamps(&[1000000, 2000000, 3000000])
//!     .add_f64_column("open", &[100.0, 101.0, 102.0])
//!     .add_f64_column("close", &[101.0, 102.0, 103.0])
//!     .add_i64_column("volume", &[1000, 1100, 1200])
//!     .build();
//! ```

use crate::binary_format::{
    BinaryDataHeader, BinaryFormatError, ColumnDescriptor, ColumnType, DATA_ALIGNMENT, StringEntry,
    align_up,
};

/// Column data being built
enum ColumnData {
    Float64(Vec<f64>),
    Int64(Vec<i64>),
    String(Vec<String>),
    Bool(Vec<bool>),
    Timestamp(Vec<i64>),
}

impl ColumnData {
    fn len(&self) -> usize {
        match self {
            Self::Float64(v) => v.len(),
            Self::Int64(v) => v.len(),
            Self::String(v) => v.len(),
            Self::Bool(v) => v.len(),
            Self::Timestamp(v) => v.len(),
        }
    }

    fn column_type(&self) -> ColumnType {
        match self {
            Self::Float64(_) => ColumnType::Float64,
            Self::Int64(_) => ColumnType::Int64,
            Self::String(_) => ColumnType::String,
            Self::Bool(_) => ColumnType::Bool,
            Self::Timestamp(_) => ColumnType::Timestamp,
        }
    }
}

/// Builder for binary columnar data
pub struct BinaryDataBuilder {
    columns: Vec<(String, ColumnData)>,
    has_timestamps: bool,
    is_sorted: bool,
}

impl Default for BinaryDataBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl BinaryDataBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            has_timestamps: false,
            is_sorted: false,
        }
    }

    /// Add a timestamp column (must be first column if used)
    pub fn add_timestamps(&mut self, values: &[i64]) -> &mut Self {
        // Timestamps should be first
        if self.columns.is_empty() {
            self.columns.push((
                "timestamp".to_string(),
                ColumnData::Timestamp(values.to_vec()),
            ));
            self.has_timestamps = true;
        } else {
            // Insert at beginning if columns already exist
            self.columns.insert(
                0,
                (
                    "timestamp".to_string(),
                    ColumnData::Timestamp(values.to_vec()),
                ),
            );
            self.has_timestamps = true;
        }
        self
    }

    /// Mark data as sorted by timestamp
    pub fn set_sorted(&mut self, sorted: bool) -> &mut Self {
        self.is_sorted = sorted;
        self
    }

    /// Add an f64 column
    pub fn add_f64_column(&mut self, name: &str, values: &[f64]) -> &mut Self {
        self.columns
            .push((name.to_string(), ColumnData::Float64(values.to_vec())));
        self
    }

    /// Add an i64 column
    pub fn add_i64_column(&mut self, name: &str, values: &[i64]) -> &mut Self {
        self.columns
            .push((name.to_string(), ColumnData::Int64(values.to_vec())));
        self
    }

    /// Add a string column
    pub fn add_string_column(&mut self, name: &str, values: &[String]) -> &mut Self {
        self.columns
            .push((name.to_string(), ColumnData::String(values.to_vec())));
        self
    }

    /// Add a bool column
    pub fn add_bool_column(&mut self, name: &str, values: &[bool]) -> &mut Self {
        self.columns
            .push((name.to_string(), ColumnData::Bool(values.to_vec())));
        self
    }

    /// Validate that all columns have the same length
    fn validate(&self) -> Result<u64, BinaryFormatError> {
        if self.columns.is_empty() {
            return Ok(0);
        }

        let first_len = self.columns[0].1.len();
        for (_name, col) in &self.columns[1..] {
            if col.len() != first_len {
                return Err(BinaryFormatError::InsufficientData {
                    expected: first_len,
                    actual: col.len(),
                });
            }
        }

        Ok(first_len as u64)
    }

    /// Build the binary data
    pub fn build(&self) -> Result<Vec<u8>, BinaryFormatError> {
        let row_count = self.validate()?;
        let column_count = self.columns.len() as u16;

        // Calculate sizes
        let header_size = BinaryDataHeader::SIZE;
        let descriptors_size = column_count as usize * ColumnDescriptor::SIZE;

        // Build string table for column names
        let mut string_table = Vec::new();
        let mut name_offsets = Vec::with_capacity(self.columns.len());
        for (name, _) in &self.columns {
            name_offsets.push(string_table.len() as u16);
            string_table.extend_from_slice(name.as_bytes());
            string_table.push(0); // null terminator
        }

        // Calculate where data section starts (aligned)
        let metadata_size = header_size + descriptors_size + string_table.len();
        let data_section_start = align_up(metadata_size, DATA_ALIGNMENT);
        let _padding_size = data_section_start - metadata_size;

        // Calculate column data offsets and sizes
        let mut column_offsets = Vec::with_capacity(self.columns.len());
        let mut current_offset = 0usize;

        for (_, col) in &self.columns {
            // Align each column to DATA_ALIGNMENT
            current_offset = align_up(current_offset, DATA_ALIGNMENT);
            column_offsets.push(current_offset);

            let col_size = match col {
                ColumnData::Float64(v) => v.len() * 8,
                ColumnData::Timestamp(v) => v.len() * 8,
                ColumnData::Int64(v) => v.len() * 8,
                ColumnData::String(v) => {
                    // String table: entries + pool
                    let entries_size = v.len() * StringEntry::SIZE;
                    let pool_size: usize = v.iter().map(|s| s.len()).sum();
                    entries_size + pool_size
                }
                ColumnData::Bool(v) => v.len(),
            };
            current_offset += col_size;
        }

        let total_data_size = current_offset;
        let total_size = data_section_start + total_data_size;

        // Build the buffer
        let mut buffer = vec![0u8; total_size];

        // Write header
        let header =
            BinaryDataHeader::new(column_count, row_count, self.has_timestamps, self.is_sorted);
        buffer[..header_size].copy_from_slice(&header.to_bytes());

        // Write column descriptors
        let mut desc_offset = header_size;
        for (i, ((_, col), &name_offset)) in self.columns.iter().zip(&name_offsets).enumerate() {
            let col_type = col.column_type();
            let data_offset = column_offsets[i] as u64;
            let data_len = match col {
                ColumnData::Float64(v) => (v.len() * 8) as u64,
                ColumnData::Timestamp(v) => (v.len() * 8) as u64,
                ColumnData::Int64(v) => (v.len() * 8) as u64,
                ColumnData::String(v) => {
                    let entries_size = v.len() * StringEntry::SIZE;
                    let pool_size: usize = v.iter().map(|s| s.len()).sum();
                    (entries_size + pool_size) as u64
                }
                ColumnData::Bool(v) => v.len() as u64,
            };

            let desc = ColumnDescriptor::new(name_offset, col_type, data_offset, data_len, false);
            let desc_bytes: [u8; ColumnDescriptor::SIZE] = unsafe { std::mem::transmute(desc) };
            buffer[desc_offset..desc_offset + ColumnDescriptor::SIZE].copy_from_slice(&desc_bytes);
            desc_offset += ColumnDescriptor::SIZE;
        }

        // Write string table (column names)
        buffer[desc_offset..desc_offset + string_table.len()].copy_from_slice(&string_table);

        // Write padding (already zeroed)

        // Write column data
        for (i, (_, col)) in self.columns.iter().enumerate() {
            let offset = data_section_start + column_offsets[i];

            match col {
                ColumnData::Float64(v) => {
                    let bytes =
                        unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 8) };
                    buffer[offset..offset + bytes.len()].copy_from_slice(bytes);
                }
                ColumnData::Int64(v) => {
                    let bytes =
                        unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 8) };
                    buffer[offset..offset + bytes.len()].copy_from_slice(bytes);
                }
                ColumnData::Timestamp(v) => {
                    let bytes =
                        unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 8) };
                    buffer[offset..offset + bytes.len()].copy_from_slice(bytes);
                }
                ColumnData::String(v) => {
                    // Write string entries
                    let entries_size = v.len() * StringEntry::SIZE;
                    let mut pool_offset = 0u32;
                    let mut entry_offset = offset;

                    for s in v {
                        let entry = StringEntry {
                            offset: pool_offset,
                            length: s.len() as u32,
                        };
                        let entry_bytes: [u8; StringEntry::SIZE] =
                            unsafe { std::mem::transmute(entry) };
                        buffer[entry_offset..entry_offset + StringEntry::SIZE]
                            .copy_from_slice(&entry_bytes);
                        entry_offset += StringEntry::SIZE;
                        pool_offset += s.len() as u32;
                    }

                    // Write string pool
                    let pool_start = offset + entries_size;
                    let mut pool_pos = pool_start;
                    for s in v {
                        buffer[pool_pos..pool_pos + s.len()].copy_from_slice(s.as_bytes());
                        pool_pos += s.len();
                    }
                }
                ColumnData::Bool(v) => {
                    for (j, &b) in v.iter().enumerate() {
                        buffer[offset + j] = if b { 1 } else { 0 };
                    }
                }
            }
        }

        Ok(buffer)
    }

    /// Build and return as a boxed slice (for FFI)
    pub fn build_boxed(&self) -> Result<Box<[u8]>, BinaryFormatError> {
        self.build().map(|v| v.into_boxed_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary_format::{BINARY_MAGIC, BinaryDataHeader};

    #[test]
    fn test_build_simple() {
        let data = BinaryDataBuilder::new()
            .add_timestamps(&[1000000, 2000000, 3000000])
            .add_f64_column("value", &[1.0, 2.0, 3.0])
            .build()
            .unwrap();

        // Parse header (use safe accessors for packed struct)
        let header = BinaryDataHeader::from_bytes(&data).unwrap();
        assert_eq!(header.get_magic(), BINARY_MAGIC);
        assert_eq!(header.get_column_count(), 2);
        assert_eq!(header.get_row_count(), 3);
        assert!(header.has_timestamps());
    }

    #[test]
    fn test_build_with_strings() {
        let data = BinaryDataBuilder::new()
            .add_timestamps(&[1000000, 2000000])
            .add_string_column("name", &["foo".to_string(), "bar".to_string()])
            .add_f64_column("value", &[1.0, 2.0])
            .build()
            .unwrap();

        let header = BinaryDataHeader::from_bytes(&data).unwrap();
        assert_eq!(header.get_column_count(), 3);
        assert_eq!(header.get_row_count(), 2);
    }

    #[test]
    fn test_build_empty() {
        let data = BinaryDataBuilder::new().build().unwrap();

        let header = BinaryDataHeader::from_bytes(&data).unwrap();
        assert_eq!(header.get_column_count(), 0);
        assert_eq!(header.get_row_count(), 0);
    }

    #[test]
    fn test_validate_mismatched_lengths() {
        let result = BinaryDataBuilder::new()
            .add_timestamps(&[1000000, 2000000, 3000000])
            .add_f64_column("value", &[1.0, 2.0]) // Wrong length
            .validate();

        assert!(result.is_err());
    }
}
