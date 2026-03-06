//! Transparent zstd compression for wire frames.
//!
//! Frame format: `[flags: u8] [body...]`
//! - `flags & 0x01` = body is zstd compressed
//! - Payloads < COMPRESSION_THRESHOLD bytes: stored uncompressed
//! - Payloads >= threshold: compressed with zstd level 3, used only if smaller

use super::TransportError;

/// Minimum payload size to attempt compression.
pub const COMPRESSION_THRESHOLD: usize = 256;

/// Maximum allowed decompressed size (256 MB) to prevent decompression bombs.
pub const MAX_DECOMPRESSED_SIZE: usize = 256 * 1024 * 1024;

/// Compression level for zstd (level 3 = good ratio with fast speed).
const ZSTD_LEVEL: i32 = 3;

const FLAG_COMPRESSED: u8 = 0x01;

/// Encode data into a framed payload: `[flags: u8] [body...]`
///
/// If data is >= COMPRESSION_THRESHOLD bytes and compresses smaller,
/// the body is zstd-compressed and FLAG_COMPRESSED is set.
pub fn encode_framed(data: &[u8]) -> Vec<u8> {
    if data.len() < COMPRESSION_THRESHOLD {
        let mut out = Vec::with_capacity(1 + data.len());
        out.push(0x00); // no compression
        out.extend_from_slice(data);
        return out;
    }

    match zstd::stream::encode_all(data, ZSTD_LEVEL) {
        Ok(compressed) if compressed.len() < data.len() => {
            let mut out = Vec::with_capacity(1 + compressed.len());
            out.push(FLAG_COMPRESSED);
            out.extend_from_slice(&compressed);
            out
        }
        _ => {
            // Compression failed or didn't help — store raw
            let mut out = Vec::with_capacity(1 + data.len());
            out.push(0x00);
            out.extend_from_slice(data);
            out
        }
    }
}

/// Decode a framed payload: read flags byte, decompress if needed.
pub fn decode_framed(data: &[u8]) -> Result<Vec<u8>, TransportError> {
    if data.is_empty() {
        return Err(TransportError::ReceiveFailed(
            "empty framed payload".to_string(),
        ));
    }

    let flags = data[0];
    let body = &data[1..];

    if flags & FLAG_COMPRESSED != 0 {
        let decompressed = zstd::stream::decode_all(body)
            .map_err(|e| TransportError::ReceiveFailed(format!("zstd decompress: {}", e)))?;

        if decompressed.len() > MAX_DECOMPRESSED_SIZE {
            return Err(TransportError::PayloadTooLarge {
                size: decompressed.len(),
                max: MAX_DECOMPRESSED_SIZE,
            });
        }

        Ok(decompressed)
    } else {
        Ok(body.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_small_payload_no_compression() {
        let data = b"hello";
        let framed = encode_framed(data);
        assert_eq!(framed[0], 0x00, "small payload should not be compressed");
        assert_eq!(&framed[1..], data);
        let decoded = decode_framed(&framed).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_large_compressible_payload() {
        // Highly compressible: repeated pattern
        let data = vec![0x42u8; 4096];
        let framed = encode_framed(&data);
        assert_eq!(
            framed[0] & FLAG_COMPRESSED,
            FLAG_COMPRESSED,
            "large compressible payload should be compressed"
        );
        assert!(framed.len() < data.len(), "compressed should be smaller");
        let decoded = decode_framed(&framed).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_large_incompressible_payload() {
        // Random-ish data that won't compress well
        let mut data = Vec::with_capacity(1024);
        for i in 0..1024u32 {
            data.extend_from_slice(&i.to_le_bytes());
        }
        let framed = encode_framed(&data);
        let decoded = decode_framed(&framed).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_empty_framed_payload_error() {
        assert!(decode_framed(&[]).is_err());
    }

    #[test]
    fn test_roundtrip_at_threshold_boundary() {
        let data = vec![0xAA; COMPRESSION_THRESHOLD];
        let framed = encode_framed(&data);
        let decoded = decode_framed(&framed).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_just_below_threshold() {
        let data = vec![0xBB; COMPRESSION_THRESHOLD - 1];
        let framed = encode_framed(&data);
        assert_eq!(framed[0], 0x00, "below threshold should not compress");
        let decoded = decode_framed(&framed).unwrap();
        assert_eq!(decoded, data);
    }
}
