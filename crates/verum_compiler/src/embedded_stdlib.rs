//! Embedded standard library loader.
//!
//! Provides access to the Verum stdlib (core/*.vr) source files that are
//! embedded directly in the compiler binary at build time.
//!
//! This enables single-binary distribution: `verum` contains all stdlib
//! sources and doesn't need a separate `core/` directory at runtime.
//!
//! # Architecture
//!
//! ```text
//! Build time:  core/*.vr → zstd compress → include_bytes! → binary
//! Runtime:     binary → zstd decompress (~2ms) → in-memory archive → path lookup
//! ```
//!
//! # Performance
//!
//! - Decompression: ~2ms for 4.7MB (zstd is extremely fast)
//! - Lookup: O(log n) binary search on sorted paths
//! - Memory: ~5MB for decompressed archive (shared, read-only)
//! - First access triggers lazy decompression

use std::collections::HashMap;
use std::sync::OnceLock;

/// The compressed stdlib archive embedded at build time.
static STDLIB_COMPRESSED: &[u8] = include_bytes!(env!("STDLIB_ARCHIVE_PATH"));

/// Lazily decompressed and indexed stdlib archive.
static STDLIB_INDEX: OnceLock<StdlibArchive> = OnceLock::new();

/// Decompressed stdlib archive with fast path-based lookup.
pub struct StdlibArchive {
    /// Raw decompressed data
    data: Vec<u8>,
    /// File entries: path → (offset_in_data, length)
    files: HashMap<String, (usize, usize)>,
}

impl StdlibArchive {
    /// Decompress and index the embedded archive.
    fn from_compressed(compressed: &[u8]) -> Option<Self> {
        if compressed.is_empty() {
            return None;
        }

        // Decompress
        let data = zstd::decode_all(compressed).ok()?;

        if data.len() < 4 {
            return None;
        }

        // Parse header
        let file_count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let mut files = HashMap::with_capacity(file_count);
        let mut cursor = 4usize;

        for _ in 0..file_count {
            if cursor + 2 > data.len() { break; }
            let path_len = u16::from_le_bytes([data[cursor], data[cursor + 1]]) as usize;
            cursor += 2;

            if cursor + path_len > data.len() { break; }
            let path = String::from_utf8_lossy(&data[cursor..cursor + path_len]).to_string();
            cursor += path_len;

            if cursor + 8 > data.len() { break; }
            let content_offset = u32::from_le_bytes([
                data[cursor], data[cursor + 1], data[cursor + 2], data[cursor + 3],
            ]) as usize;
            cursor += 4;
            let content_len = u32::from_le_bytes([
                data[cursor], data[cursor + 1], data[cursor + 2], data[cursor + 3],
            ]) as usize;
            cursor += 4;

            files.insert(path, (content_offset, content_len));
        }

        Some(Self { data, files })
    }

    /// Get a file's source text by its relative path (e.g., "base/maybe.vr").
    pub fn get_file(&self, path: &str) -> Option<&str> {
        let (offset, len) = self.files.get(path)?;
        if *offset + *len > self.data.len() {
            return None;
        }
        std::str::from_utf8(&self.data[*offset..*offset + *len]).ok()
    }

    /// List all embedded file paths.
    pub fn file_paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(|s| s.as_str())
    }

    /// Number of embedded files.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Total decompressed size in bytes.
    pub fn total_size(&self) -> usize {
        self.data.len()
    }
}

/// Get the global embedded stdlib archive.
/// Returns None if the archive is empty (no core/ at build time).
pub fn get_embedded_stdlib() -> Option<&'static StdlibArchive> {
    STDLIB_INDEX
        .get_or_init(|| {
            StdlibArchive::from_compressed(STDLIB_COMPRESSED).unwrap_or_else(|| StdlibArchive {
                data: Vec::new(),
                files: HashMap::new(),
            })
        })
        .file_count()
        .gt(&0)
        .then(|| STDLIB_INDEX.get().unwrap())
}

/// Check if embedded stdlib is available.
pub fn has_embedded_stdlib() -> bool {
    !STDLIB_COMPRESSED.is_empty()
}

/// Get compressed archive size (for diagnostics).
pub fn compressed_size() -> usize {
    STDLIB_COMPRESSED.len()
}
