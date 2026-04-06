//! VBC compression support.
//!
//! This module provides compression and decompression for VBC sections
//! using zstd (default) or lz4 algorithms. Compression is optional and
//! controlled by the `compression` feature flag.

use crate::error::{VbcError, VbcResult};
use crate::format::CompressionAlgorithm;

/// Default zstd compression level (3 = fast with good ratio).
pub const ZSTD_DEFAULT_LEVEL: i32 = 3;

/// Minimum size threshold for compression (sections smaller than this are not compressed).
/// Set to 512 bytes - below this the overhead isn't worth it.
pub const MIN_COMPRESS_SIZE: usize = 512;

/// Compression options for VBC serialization.
#[derive(Debug, Clone, Copy)]
pub struct CompressionOptions {
    /// Algorithm to use.
    pub algorithm: CompressionAlgorithm,
    /// Compression level (algorithm-specific).
    pub level: i32,
    /// Minimum size for compression (sections smaller are stored uncompressed).
    pub min_size: usize,
}

impl Default for CompressionOptions {
    fn default() -> Self {
        Self {
            algorithm: CompressionAlgorithm::Zstd,
            level: ZSTD_DEFAULT_LEVEL,
            min_size: MIN_COMPRESS_SIZE,
        }
    }
}

impl CompressionOptions {
    /// Create options for no compression.
    pub fn none() -> Self {
        Self {
            algorithm: CompressionAlgorithm::None,
            level: 0,
            min_size: usize::MAX,
        }
    }

    /// Create options for zstd compression with default level.
    pub fn zstd() -> Self {
        Self::default()
    }

    /// Create options for zstd compression with custom level.
    pub fn zstd_level(level: i32) -> Self {
        Self {
            algorithm: CompressionAlgorithm::Zstd,
            level,
            min_size: MIN_COMPRESS_SIZE,
        }
    }

    /// Create options for lz4 compression (faster, lower ratio).
    pub fn lz4() -> Self {
        Self {
            algorithm: CompressionAlgorithm::Lz4,
            level: 0, // LZ4 doesn't use level
            min_size: MIN_COMPRESS_SIZE,
        }
    }
}

/// Compress data using the specified algorithm.
///
/// Returns `None` if compression is disabled or if the data is smaller than
/// the minimum threshold. Returns `Some((compressed_data, algorithm))` on success.
#[cfg(feature = "compression")]
pub fn compress(data: &[u8], options: &CompressionOptions) -> VbcResult<Option<(Vec<u8>, CompressionAlgorithm)>> {
    // Skip compression for small data or when disabled
    if options.algorithm == CompressionAlgorithm::None || data.len() < options.min_size {
        return Ok(None);
    }

    let compressed = match options.algorithm {
        CompressionAlgorithm::None => return Ok(None),
        CompressionAlgorithm::Zstd => {
            zstd::encode_all(data, options.level)
                .map_err(|e| VbcError::Compression(format!("zstd compress: {}", e)))?
        }
        CompressionAlgorithm::Lz4 => {
            lz4_flex::compress_prepend_size(data)
        }
    };

    // Only use compression if it actually reduces size
    if compressed.len() >= data.len() {
        return Ok(None);
    }

    Ok(Some((compressed, options.algorithm)))
}

/// Compress data (stub when compression feature is disabled).
#[cfg(not(feature = "compression"))]
pub fn compress(_data: &[u8], _options: &CompressionOptions) -> VbcResult<Option<(Vec<u8>, CompressionAlgorithm)>> {
    Ok(None)
}

/// Decompress data using the specified algorithm.
///
/// # Arguments
/// * `data` - The compressed data
/// * `algorithm` - The algorithm used for compression
/// * `uncompressed_size` - Expected size of decompressed data (for validation)
#[cfg(feature = "compression")]
pub fn decompress(data: &[u8], algorithm: CompressionAlgorithm, uncompressed_size: u32) -> VbcResult<Vec<u8>> {
    match algorithm {
        CompressionAlgorithm::None => Ok(data.to_vec()),
        CompressionAlgorithm::Zstd => {
            let mut result = Vec::with_capacity(uncompressed_size as usize);
            let mut decoder = zstd::Decoder::new(data)
                .map_err(|e| VbcError::Decompression(format!("zstd init: {}", e)))?;
            std::io::copy(&mut decoder, &mut result)
                .map_err(|e| VbcError::Decompression(format!("zstd decompress: {}", e)))?;

            if result.len() != uncompressed_size as usize {
                return Err(VbcError::Decompression(format!(
                    "zstd size mismatch: expected {}, got {}",
                    uncompressed_size,
                    result.len()
                )));
            }
            Ok(result)
        }
        CompressionAlgorithm::Lz4 => {
            let result = lz4_flex::decompress_size_prepended(data)
                .map_err(|e| VbcError::Decompression(format!("lz4 decompress: {}", e)))?;

            if result.len() != uncompressed_size as usize {
                return Err(VbcError::Decompression(format!(
                    "lz4 size mismatch: expected {}, got {}",
                    uncompressed_size,
                    result.len()
                )));
            }
            Ok(result)
        }
    }
}

/// Decompress data (stub when compression feature is disabled).
#[cfg(not(feature = "compression"))]
pub fn decompress(data: &[u8], algorithm: CompressionAlgorithm, _uncompressed_size: u32) -> VbcResult<Vec<u8>> {
    match algorithm {
        CompressionAlgorithm::None => Ok(data.to_vec()),
        _ => Err(VbcError::Decompression(
            "compression feature not enabled".to_string()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_options_default() {
        let opts = CompressionOptions::default();
        assert_eq!(opts.algorithm, CompressionAlgorithm::Zstd);
        assert_eq!(opts.level, ZSTD_DEFAULT_LEVEL);
    }

    #[test]
    fn test_compression_options_none() {
        let opts = CompressionOptions::none();
        assert_eq!(opts.algorithm, CompressionAlgorithm::None);
    }

    #[cfg(feature = "compression")]
    #[test]
    fn test_zstd_roundtrip() {
        let original = vec![0u8; 1024]; // Compressible data
        let opts = CompressionOptions::zstd();

        let result = compress(&original, &opts).unwrap();
        assert!(result.is_some(), "Should compress zeros");

        let (compressed, algo) = result.unwrap();
        assert_eq!(algo, CompressionAlgorithm::Zstd);
        assert!(compressed.len() < original.len(), "Should reduce size");

        let decompressed = decompress(&compressed, algo, original.len() as u32).unwrap();
        assert_eq!(decompressed, original);
    }

    #[cfg(feature = "compression")]
    #[test]
    fn test_lz4_roundtrip() {
        let original = vec![0u8; 1024]; // Compressible data
        let opts = CompressionOptions::lz4();

        let result = compress(&original, &opts).unwrap();
        assert!(result.is_some(), "Should compress zeros");

        let (compressed, algo) = result.unwrap();
        assert_eq!(algo, CompressionAlgorithm::Lz4);

        let decompressed = decompress(&compressed, algo, original.len() as u32).unwrap();
        assert_eq!(decompressed, original);
    }

    #[cfg(feature = "compression")]
    #[test]
    fn test_skip_small_data() {
        let small = vec![1, 2, 3, 4, 5];
        let opts = CompressionOptions::default();

        let result = compress(&small, &opts).unwrap();
        assert!(result.is_none(), "Should skip small data");
    }

    #[cfg(feature = "compression")]
    #[test]
    fn test_skip_incompressible_data() {
        // Random-ish data that doesn't compress well
        let data: Vec<u8> = (0..1024).map(|i| (i * 17 + 31) as u8).collect();
        let mut opts = CompressionOptions::zstd();
        opts.min_size = 0; // Force compression attempt

        let result = compress(&data, &opts).unwrap();
        // May or may not compress depending on data pattern
        // The important thing is it doesn't panic
        if let Some((compressed, _)) = result {
            // If it did compress, verify roundtrip
            let decompressed = decompress(&compressed, CompressionAlgorithm::Zstd, data.len() as u32).unwrap();
            assert_eq!(decompressed, data);
        }
    }
}
