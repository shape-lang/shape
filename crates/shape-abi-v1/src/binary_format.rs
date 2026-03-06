//! Binary columnar data format for high-performance data loading
//!
//! This format allows plugins to return data that can be directly mapped to
//! SeriesStorage without JSON intermediate conversions.
//!
//! # Format Layout
//!
//! ```text
//! +------------------+
//! | Header (16 bytes)|
//! +------------------+
//! | Column Descriptors |  (20 bytes each)
//! +------------------+
//! | String Table     |  (column names, null-terminated)
//! +------------------+
//! | Padding (align)  |  (to 8-byte boundary)
//! +------------------+
//! | Column Data      |  (aligned to 32 bytes for SIMD)
//! +------------------+
//! ```

use std::mem::size_of;

/// Magic number for binary data format: "LAMI" in little-endian
pub const BINARY_MAGIC: u32 = 0x494D414C; // "IMAL" reversed = "LAMI"

/// Current binary format version
pub const BINARY_FORMAT_VERSION: u8 = 1;

/// Alignment for column data (32 bytes for AVX2 SIMD)
pub const DATA_ALIGNMENT: usize = 32;

/// Header flags
pub mod flags {
    /// Data has a timestamp column (first column is always timestamp if set)
    pub const HAS_TIMESTAMPS: u8 = 0x01;
    /// Data is sorted by timestamp
    pub const IS_SORTED: u8 = 0x02;
}

/// Binary data header (16 bytes, fixed size)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BinaryDataHeader {
    /// Magic number for validation (0x494D414C = "LAMI")
    pub magic: u32,
    /// Format version
    pub version: u8,
    /// Flags (HAS_TIMESTAMPS, IS_SORTED)
    pub flags: u8,
    /// Number of columns
    pub column_count: u16,
    /// Total number of rows
    pub row_count: u64,
}

impl BinaryDataHeader {
    pub const SIZE: usize = size_of::<Self>();

    /// Create a new header
    pub fn new(column_count: u16, row_count: u64, has_timestamps: bool, is_sorted: bool) -> Self {
        let mut header_flags = 0u8;
        if has_timestamps {
            header_flags |= flags::HAS_TIMESTAMPS;
        }
        if is_sorted {
            header_flags |= flags::IS_SORTED;
        }

        Self {
            magic: BINARY_MAGIC,
            version: BINARY_FORMAT_VERSION,
            flags: header_flags,
            column_count,
            row_count,
        }
    }

    /// Validate the header
    pub fn validate(&self) -> Result<(), BinaryFormatError> {
        if self.magic != BINARY_MAGIC {
            return Err(BinaryFormatError::InvalidMagic(self.magic));
        }
        if self.version != BINARY_FORMAT_VERSION {
            return Err(BinaryFormatError::UnsupportedVersion(self.version));
        }
        Ok(())
    }

    /// Check if data has timestamps
    pub fn has_timestamps(&self) -> bool {
        self.flags & flags::HAS_TIMESTAMPS != 0
    }

    /// Check if data is sorted
    pub fn is_sorted(&self) -> bool {
        self.flags & flags::IS_SORTED != 0
    }

    /// Get magic number (safe accessor for packed struct)
    pub fn get_magic(&self) -> u32 {
        // Use addr_of! to avoid creating a reference to packed field
        unsafe { std::ptr::read_unaligned(std::ptr::addr_of!(self.magic)) }
    }

    /// Get column count (safe accessor for packed struct)
    pub fn get_column_count(&self) -> u16 {
        unsafe { std::ptr::read_unaligned(std::ptr::addr_of!(self.column_count)) }
    }

    /// Get row count (safe accessor for packed struct)
    pub fn get_row_count(&self) -> u64 {
        unsafe { std::ptr::read_unaligned(std::ptr::addr_of!(self.row_count)) }
    }

    /// Read header from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, BinaryFormatError> {
        if data.len() < Self::SIZE {
            return Err(BinaryFormatError::InsufficientData {
                expected: Self::SIZE,
                actual: data.len(),
            });
        }

        // Safe to read since we checked the length
        let header = unsafe { std::ptr::read_unaligned(data.as_ptr() as *const Self) };
        header.validate()?;
        Ok(header)
    }

    /// Write header to bytes
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        unsafe { std::mem::transmute_copy(self) }
    }
}

/// Column data type
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    /// f64 (8 bytes per element)
    Float64 = 0,
    /// i64 (8 bytes per element)
    Int64 = 1,
    /// String (variable length, stored as offset/length pairs + string pool)
    String = 2,
    /// bool (1 byte per element)
    Bool = 3,
    /// Timestamp in microseconds (i64, alias for Int64 with semantic meaning)
    Timestamp = 4,
}

impl ColumnType {
    /// Convert from u8
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Float64),
            1 => Some(Self::Int64),
            2 => Some(Self::String),
            3 => Some(Self::Bool),
            4 => Some(Self::Timestamp),
            _ => None,
        }
    }

    /// Element size in bytes (0 for variable-length types)
    pub fn element_size(&self) -> usize {
        match self {
            Self::Float64 | Self::Int64 | Self::Timestamp => 8,
            Self::Bool => 1,
            Self::String => 0, // Variable length
        }
    }

    /// Whether this type has fixed-size elements
    pub fn is_fixed_size(&self) -> bool {
        !matches!(self, Self::String)
    }
}

/// Column descriptor flags
pub mod col_flags {
    /// Column may contain null/NaN values
    pub const NULLABLE: u8 = 0x01;
    /// Column data is pre-sorted
    pub const SORTED: u8 = 0x02;
}

/// Column descriptor (20 bytes)
///
/// For fixed-size types (Float64, Int64, Bool, Timestamp):
/// - `data_offset`: Byte offset to raw array data
/// - `data_len`: Total bytes = row_count * element_size
///
/// For String type:
/// - `data_offset`: Byte offset to string table header (offsets + lengths)
/// - `data_len`: Total bytes of string table (header + string pool)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ColumnDescriptor {
    /// Offset to column name in string table (from start of string table section)
    pub name_offset: u16,
    /// Column data type
    pub data_type: u8,
    /// Column flags (nullable, sorted)
    pub flags: u8,
    /// Offset to column data (from start of data section)
    pub data_offset: u64,
    /// Length of column data in bytes
    pub data_len: u64,
}

impl ColumnDescriptor {
    pub const SIZE: usize = size_of::<Self>();

    /// Create a new column descriptor
    pub fn new(
        name_offset: u16,
        data_type: ColumnType,
        data_offset: u64,
        data_len: u64,
        nullable: bool,
    ) -> Self {
        let mut column_flags = 0u8;
        if nullable {
            column_flags |= col_flags::NULLABLE;
        }

        Self {
            name_offset,
            data_type: data_type as u8,
            flags: column_flags,
            data_offset,
            data_len,
        }
    }

    /// Get the column type
    pub fn column_type(&self) -> Option<ColumnType> {
        ColumnType::from_u8(self.data_type)
    }

    /// Check if column is nullable
    pub fn is_nullable(&self) -> bool {
        self.flags & col_flags::NULLABLE != 0
    }
}

/// String entry header for string columns
///
/// String columns store data as:
/// 1. Array of StringEntry (offset + length) for each row
/// 2. Pool of concatenated string bytes
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct StringEntry {
    /// Offset into string pool (from start of pool)
    pub offset: u32,
    /// Length of string in bytes
    pub length: u32,
}

impl StringEntry {
    pub const SIZE: usize = size_of::<Self>();
}

/// Errors when parsing binary format
#[derive(Debug, Clone, PartialEq)]
pub enum BinaryFormatError {
    /// Invalid magic number
    InvalidMagic(u32),
    /// Unsupported format version
    UnsupportedVersion(u8),
    /// Not enough data
    InsufficientData { expected: usize, actual: usize },
    /// Invalid column type
    InvalidColumnType(u8),
    /// Column name not found
    ColumnNameNotFound { offset: u16 },
    /// Data alignment error
    AlignmentError { offset: usize, required: usize },
    /// Invalid string encoding
    InvalidUtf8,
}

impl std::fmt::Display for BinaryFormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMagic(m) => write!(f, "Invalid magic number: 0x{:08X}", m),
            Self::UnsupportedVersion(v) => write!(f, "Unsupported binary format version: {}", v),
            Self::InsufficientData { expected, actual } => {
                write!(
                    f,
                    "Insufficient data: expected {} bytes, got {}",
                    expected, actual
                )
            }
            Self::InvalidColumnType(t) => write!(f, "Invalid column type: {}", t),
            Self::ColumnNameNotFound { offset } => {
                write!(f, "Column name not found at offset {}", offset)
            }
            Self::AlignmentError { offset, required } => {
                write!(
                    f,
                    "Data at offset {} not aligned to {} bytes",
                    offset, required
                )
            }
            Self::InvalidUtf8 => write!(f, "Invalid UTF-8 string encoding"),
        }
    }
}

impl std::error::Error for BinaryFormatError {}

/// Calculate padding needed to align to given boundary
pub const fn align_padding(offset: usize, alignment: usize) -> usize {
    let remainder = offset % alignment;
    if remainder == 0 {
        0
    } else {
        alignment - remainder
    }
}

/// Align offset to given boundary
pub const fn align_up(offset: usize, alignment: usize) -> usize {
    offset + align_padding(offset, alignment)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_size() {
        assert_eq!(BinaryDataHeader::SIZE, 16);
    }

    #[test]
    fn test_column_descriptor_size() {
        assert_eq!(ColumnDescriptor::SIZE, 20);
    }

    #[test]
    fn test_header_roundtrip() {
        let header = BinaryDataHeader::new(5, 1000, true, true);
        let bytes = header.to_bytes();
        let parsed = BinaryDataHeader::from_bytes(&bytes).unwrap();

        // Use safe accessor methods for packed struct
        assert_eq!(parsed.get_magic(), BINARY_MAGIC);
        assert_eq!(parsed.get_column_count(), 5);
        assert_eq!(parsed.get_row_count(), 1000);
        assert!(parsed.has_timestamps());
        assert!(parsed.is_sorted());
    }

    #[test]
    fn test_column_type_element_size() {
        assert_eq!(ColumnType::Float64.element_size(), 8);
        assert_eq!(ColumnType::Int64.element_size(), 8);
        assert_eq!(ColumnType::Timestamp.element_size(), 8);
        assert_eq!(ColumnType::Bool.element_size(), 1);
        assert_eq!(ColumnType::String.element_size(), 0);
    }

    #[test]
    fn test_alignment() {
        assert_eq!(align_up(0, 32), 0);
        assert_eq!(align_up(1, 32), 32);
        assert_eq!(align_up(31, 32), 32);
        assert_eq!(align_up(32, 32), 32);
        assert_eq!(align_up(33, 32), 64);
    }
}
