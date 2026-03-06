//! Async file I/O operations for the io module.
//!
//! Provides non-blocking file read/write using tokio's async file API.
//! These integrate with the VM's async/await system via `AsyncModuleFn`.

use shape_value::ValueWord;
use std::sync::Arc;

/// io.read_file_async(path: string) -> string
///
/// Asynchronously reads the entire contents of a file as a string.
pub async fn io_read_file_async(args: Vec<ValueWord>) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.read_file_async() requires a string path argument".to_string())?
        .to_string();

    let contents = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("io.read_file_async(\"{}\"): {}", path, e))?;

    Ok(ValueWord::from_string(Arc::new(contents)))
}

/// io.write_file_async(path: string, data: string) -> int
///
/// Asynchronously writes a string to a file, creating or truncating as needed.
/// Returns the number of bytes written.
pub async fn io_write_file_async(args: Vec<ValueWord>) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.write_file_async() requires a string path argument".to_string())?
        .to_string();

    let data = args
        .get(1)
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.write_file_async() requires a string data argument".to_string())?
        .to_string();

    let bytes_written = data.len();
    tokio::fs::write(&path, &data)
        .await
        .map_err(|e| format!("io.write_file_async(\"{}\"): {}", path, e))?;

    Ok(ValueWord::from_i64(bytes_written as i64))
}

/// io.append_file_async(path: string, data: string) -> int
///
/// Asynchronously appends a string to a file.
/// Returns the number of bytes written.
pub async fn io_append_file_async(args: Vec<ValueWord>) -> Result<ValueWord, String> {
    use tokio::io::AsyncWriteExt;

    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.append_file_async() requires a string path argument".to_string())?
        .to_string();

    let data = args
        .get(1)
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.append_file_async() requires a string data argument".to_string())?
        .to_string();

    let bytes_written = data.len();
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .await
        .map_err(|e| format!("io.append_file_async(\"{}\"): {}", path, e))?;

    file.write_all(data.as_bytes())
        .await
        .map_err(|e| format!("io.append_file_async(\"{}\"): {}", path, e))?;

    file.flush()
        .await
        .map_err(|e| format!("io.append_file_async(\"{}\"): flush: {}", path, e))?;

    Ok(ValueWord::from_i64(bytes_written as i64))
}

/// io.read_bytes_async(path: string) -> Array<int>
///
/// Asynchronously reads a file as raw bytes, returning an array of ints.
pub async fn io_read_bytes_async(args: Vec<ValueWord>) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.read_bytes_async() requires a string path argument".to_string())?
        .to_string();

    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| format!("io.read_bytes_async(\"{}\"): {}", path, e))?;

    let arr: Vec<ValueWord> = bytes
        .iter()
        .map(|&b| ValueWord::from_i64(b as i64))
        .collect();
    Ok(ValueWord::from_array(Arc::new(arr)))
}

/// io.exists_async(path: string) -> bool
///
/// Asynchronously checks if a path exists.
pub async fn io_exists_async(args: Vec<ValueWord>) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.exists_async() requires a string path argument".to_string())?
        .to_string();

    // tokio::fs doesn't have an exists() — use metadata
    let exists = tokio::fs::metadata(&path).await.is_ok();
    Ok(ValueWord::from_bool(exists))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_read_file_async() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "async hello").unwrap();

        let result = io_read_file_async(vec![ValueWord::from_string(Arc::new(
            path.to_string_lossy().to_string(),
        ))])
        .await
        .unwrap();

        assert_eq!(result.as_str(), Some("async hello"));
    }

    #[tokio::test]
    async fn test_write_file_async() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");

        let result = io_write_file_async(vec![
            ValueWord::from_string(Arc::new(path.to_string_lossy().to_string())),
            ValueWord::from_string(Arc::new("async world".to_string())),
        ])
        .await
        .unwrap();

        assert_eq!(result.as_i64(), Some(11)); // "async world".len()
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "async world");
    }

    #[tokio::test]
    async fn test_append_file_async() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "first").unwrap();

        let result = io_append_file_async(vec![
            ValueWord::from_string(Arc::new(path.to_string_lossy().to_string())),
            ValueWord::from_string(Arc::new("_second".to_string())),
        ])
        .await
        .unwrap();

        assert_eq!(result.as_i64(), Some(7)); // "_second".len()
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "first_second");
    }

    #[tokio::test]
    async fn test_read_bytes_async() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, &[0xAB, 0xCD, 0xEF]).unwrap();

        let result = io_read_bytes_async(vec![ValueWord::from_string(Arc::new(
            path.to_string_lossy().to_string(),
        ))])
        .await
        .unwrap();

        let arr = result.as_any_array().unwrap().to_generic();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_i64(), Some(0xAB));
        assert_eq!(arr[1].as_i64(), Some(0xCD));
        assert_eq!(arr[2].as_i64(), Some(0xEF));
    }

    #[tokio::test]
    async fn test_exists_async() {
        let result = io_exists_async(vec![ValueWord::from_string(Arc::new("/tmp".to_string()))])
            .await
            .unwrap();
        assert_eq!(result.as_bool(), Some(true));

        let result = io_exists_async(vec![ValueWord::from_string(Arc::new(
            "/nonexistent_xyz_test".to_string(),
        ))])
        .await
        .unwrap();
        assert_eq!(result.as_bool(), Some(false));
    }

    #[tokio::test]
    async fn test_read_file_async_missing() {
        let result = io_read_file_async(vec![ValueWord::from_string(Arc::new(
            "/nonexistent_file_xyz".to_string(),
        ))])
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_write_file_async_requires_args() {
        let result = io_write_file_async(vec![]).await;
        assert!(result.is_err());
    }
}
