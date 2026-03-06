//! Binary reader for converting binary columnar data to DataTable
//!
//! Reads binary data produced by plugins and converts it to DataTable.
//! Helper functions for reading typed columns from binary format are available
//! for use by the DataTable migration.

use std::sync::Arc;

use arrow_array::{
    ArrayRef, BooleanArray, Float64Array, Int64Array, StringArray, TimestampMicrosecondArray,
};
use arrow_schema::{DataType, Field, TimeUnit};
use shape_abi_v1::binary_format::{
    BinaryDataHeader, BinaryFormatError, ColumnDescriptor, ColumnType, StringEntry,
};
use shape_ast::error::{Result, ShapeError};
use shape_value::DataTableBuilder;

/// Read binary columnar data and convert to a DataTable.
pub fn read_binary_to_datatable(data: &[u8]) -> Result<shape_value::DataTable> {
    let header = BinaryDataHeader::from_bytes(data).map_err(format_error)?;
    let column_count = header.get_column_count() as usize;
    let row_count = header.get_row_count() as usize;

    // Read column descriptors (starts after 16-byte header)
    let desc_start = BinaryDataHeader::SIZE;
    let desc_end = desc_start + column_count * ColumnDescriptor::SIZE;
    if data.len() < desc_end {
        return Err(format_error(BinaryFormatError::InsufficientData {
            expected: desc_end,
            actual: data.len(),
        }));
    }

    let mut descriptors = Vec::with_capacity(column_count);
    for i in 0..column_count {
        let offset = desc_start + i * ColumnDescriptor::SIZE;
        let desc =
            unsafe { std::ptr::read_unaligned(data[offset..].as_ptr() as *const ColumnDescriptor) };
        descriptors.push(desc);
    }

    // String table starts after descriptors
    let string_table_start = desc_end;

    // Read column names from string table
    let mut names = Vec::with_capacity(column_count);
    for desc in &descriptors {
        let name_off = desc.name_offset as usize;
        let abs_off = string_table_start + name_off;
        // Find null terminator
        let end = data[abs_off..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| {
                format_error(BinaryFormatError::ColumnNameNotFound {
                    offset: desc.name_offset,
                })
            })?;
        let name = std::str::from_utf8(&data[abs_off..abs_off + end])
            .map_err(|_| format_error(BinaryFormatError::InvalidUtf8))?
            .to_string();
        names.push(name);
    }

    // Build Arrow fields and arrays
    let mut fields = Vec::with_capacity(column_count);
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(column_count);

    for (i, desc) in descriptors.iter().enumerate() {
        let col_type = desc
            .column_type()
            .ok_or_else(|| format_error(BinaryFormatError::InvalidColumnType(desc.data_type)))?;
        let data_offset = desc.data_offset as usize;
        let data_len = desc.data_len as usize;
        let col_data = &data[data_offset..data_offset + data_len];

        match col_type {
            ColumnType::Float64 => {
                fields.push(Field::new(&names[i], DataType::Float64, desc.is_nullable()));
                let values = read_f64_column(col_data, row_count)?;
                arrays.push(Arc::new(Float64Array::from(values)) as ArrayRef);
            }
            ColumnType::Int64 => {
                fields.push(Field::new(&names[i], DataType::Int64, desc.is_nullable()));
                let values = read_i64_column(col_data, row_count)?;
                arrays.push(Arc::new(Int64Array::from(values)) as ArrayRef);
            }
            ColumnType::String => {
                fields.push(Field::new(&names[i], DataType::Utf8, desc.is_nullable()));
                let values = read_string_column(col_data, row_count)?;
                let refs: Vec<&str> = values.iter().map(|s| s.as_str()).collect();
                arrays.push(Arc::new(StringArray::from(refs)) as ArrayRef);
            }
            ColumnType::Bool => {
                fields.push(Field::new(&names[i], DataType::Boolean, desc.is_nullable()));
                let values = read_bool_column(col_data, row_count)?;
                arrays.push(Arc::new(BooleanArray::from(values)) as ArrayRef);
            }
            ColumnType::Timestamp => {
                fields.push(Field::new(
                    &names[i],
                    DataType::Timestamp(TimeUnit::Microsecond, None),
                    desc.is_nullable(),
                ));
                let values = read_i64_column(col_data, row_count)?;
                arrays.push(Arc::new(TimestampMicrosecondArray::from(values)) as ArrayRef);
            }
        }
    }

    let mut builder = DataTableBuilder::with_fields(fields);
    for array in arrays {
        builder.add_column(array);
    }
    builder.finish().map_err(|e| ShapeError::RuntimeError {
        message: format!("Failed to build DataTable: {}", e),
        location: None,
    })
}

/// Read f64 column data
fn read_f64_column(data: &[u8], count: usize) -> Result<Vec<f64>> {
    let expected_size = count * 8;
    if data.len() < expected_size {
        return Err(format_error(BinaryFormatError::InsufficientData {
            expected: expected_size,
            actual: data.len(),
        }));
    }

    let mut values = Vec::with_capacity(count);
    for i in 0..count {
        let offset = i * 8;
        let value = f64::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]);
        values.push(value);
    }
    Ok(values)
}

/// Read i64 column data
fn read_i64_column(data: &[u8], count: usize) -> Result<Vec<i64>> {
    let expected_size = count * 8;
    if data.len() < expected_size {
        return Err(format_error(BinaryFormatError::InsufficientData {
            expected: expected_size,
            actual: data.len(),
        }));
    }

    let mut values = Vec::with_capacity(count);
    for i in 0..count {
        let offset = i * 8;
        let value = i64::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]);
        values.push(value);
    }
    Ok(values)
}

/// Read string column data
fn read_string_column(data: &[u8], count: usize) -> Result<Vec<String>> {
    // String column format:
    // [StringEntry array (count * 8 bytes)] [String pool]

    let entries_size = count * StringEntry::SIZE;
    if data.len() < entries_size {
        return Err(format_error(BinaryFormatError::InsufficientData {
            expected: entries_size,
            actual: data.len(),
        }));
    }

    let pool_start = entries_size;
    let pool = &data[pool_start..];

    let mut values = Vec::with_capacity(count);
    for i in 0..count {
        let entry_offset = i * StringEntry::SIZE;
        let entry = unsafe {
            std::ptr::read_unaligned(data[entry_offset..].as_ptr() as *const StringEntry)
        };

        let str_start = entry.offset as usize;
        let str_end = str_start + entry.length as usize;

        if str_end > pool.len() {
            return Err(format_error(BinaryFormatError::InsufficientData {
                expected: str_end,
                actual: pool.len(),
            }));
        }

        let s = std::str::from_utf8(&pool[str_start..str_end])
            .map_err(|_| format_error(BinaryFormatError::InvalidUtf8))?
            .to_string();

        values.push(s);
    }

    Ok(values)
}

/// Read bool column data
fn read_bool_column(data: &[u8], count: usize) -> Result<Vec<bool>> {
    if data.len() < count {
        return Err(format_error(BinaryFormatError::InsufficientData {
            expected: count,
            actual: data.len(),
        }));
    }

    let values: Vec<bool> = data[..count].iter().map(|&b| b != 0).collect();
    Ok(values)
}

/// Convert BinaryFormatError to ShapeError
fn format_error(e: BinaryFormatError) -> ShapeError {
    ShapeError::RuntimeError {
        message: format!("Binary format error: {}", e),
        location: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_f64_column() {
        let data: Vec<u8> = [1.0_f64, 2.0, 3.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let values = read_f64_column(&data, 3).unwrap();
        assert_eq!(values, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_read_i64_column() {
        let data: Vec<u8> = [100_i64, 200, 300]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let values = read_i64_column(&data, 3).unwrap();
        assert_eq!(values, vec![100, 200, 300]);
    }

    #[test]
    fn test_read_bool_column() {
        let data = vec![1u8, 0, 1, 1, 0];
        let values = read_bool_column(&data, 5).unwrap();
        assert_eq!(values, vec![true, false, true, true, false]);
    }

    #[test]
    fn test_read_binary_to_datatable() {
        use shape_abi_v1::binary_format::{
            BinaryDataHeader, ColumnDescriptor, ColumnType, DATA_ALIGNMENT, align_up,
        };

        let row_count: u64 = 3;
        let column_count: u16 = 2;

        // Build string table: "price\0volume\0"
        let string_table = b"price\0volume\0";
        let name_offset_price: u16 = 0;
        let name_offset_volume: u16 = 6;

        // Calculate offsets
        let desc_start = BinaryDataHeader::SIZE;
        let desc_end = desc_start + (column_count as usize) * ColumnDescriptor::SIZE;
        let string_table_end = desc_end + string_table.len();
        let data_section_start = align_up(string_table_end, DATA_ALIGNMENT);

        // Column 0: f64 "price" — 3 rows * 8 bytes = 24 bytes
        let price_data_offset = data_section_start;
        let price_data_len = 3 * 8;
        // Column 1: i64 "volume" — 3 rows * 8 bytes = 24 bytes
        let volume_data_offset = price_data_offset + price_data_len;
        let volume_data_len = 3 * 8;

        let total_size = volume_data_offset + volume_data_len;
        let mut blob = vec![0u8; total_size];

        // Write header
        let header = BinaryDataHeader::new(column_count, row_count, false, false);
        blob[..BinaryDataHeader::SIZE].copy_from_slice(&header.to_bytes());

        // Write column descriptors
        let desc0 = ColumnDescriptor::new(
            name_offset_price,
            ColumnType::Float64,
            price_data_offset as u64,
            price_data_len as u64,
            false,
        );
        let desc1 = ColumnDescriptor::new(
            name_offset_volume,
            ColumnType::Int64,
            volume_data_offset as u64,
            volume_data_len as u64,
            false,
        );
        unsafe {
            let p0 = blob[desc_start..].as_mut_ptr() as *mut ColumnDescriptor;
            std::ptr::write_unaligned(p0, desc0);
            let p1 =
                blob[desc_start + ColumnDescriptor::SIZE..].as_mut_ptr() as *mut ColumnDescriptor;
            std::ptr::write_unaligned(p1, desc1);
        }

        // Write string table
        blob[desc_end..desc_end + string_table.len()].copy_from_slice(string_table);

        // Write price column data: [100.5, 200.75, 300.0]
        let prices = [100.5_f64, 200.75, 300.0];
        for (i, v) in prices.iter().enumerate() {
            let off = price_data_offset + i * 8;
            blob[off..off + 8].copy_from_slice(&v.to_le_bytes());
        }

        // Write volume column data: [1000, 2000, 3000]
        let volumes = [1000_i64, 2000, 3000];
        for (i, v) in volumes.iter().enumerate() {
            let off = volume_data_offset + i * 8;
            blob[off..off + 8].copy_from_slice(&v.to_le_bytes());
        }

        // Parse and verify
        let dt = read_binary_to_datatable(&blob).unwrap();
        assert_eq!(dt.row_count(), 3);
        assert_eq!(dt.column_names(), vec!["price", "volume"]);

        let price_col = dt.get_f64_column("price").unwrap();
        assert_eq!(price_col.value(0), 100.5);
        assert_eq!(price_col.value(1), 200.75);
        assert_eq!(price_col.value(2), 300.0);

        let volume_col = dt.get_i64_column("volume").unwrap();
        assert_eq!(volume_col.value(0), 1000);
        assert_eq!(volume_col.value(1), 2000);
        assert_eq!(volume_col.value(2), 3000);
    }
}
