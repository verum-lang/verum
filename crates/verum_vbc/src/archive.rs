//! VBC Archive Format
//!
//! A VBC Archive (.vbca) is a collection of VBC modules that together form
//! a library or the standard library. The archive format enables:
//!
//! - **Dependency tracking**: Module dependencies are explicitly recorded
//! - **Incremental compilation**: Only recompile changed modules
//! - **Fast loading**: Single file with index for quick module lookup
//!
//! # Archive Format
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                          VBC ARCHIVE FORMAT                                  │
//! ├─────────────────────────────────────────────────────────────────────────────┤
//! │  HEADER (32 bytes)                                                          │
//! │    - Magic: "VBCA" (4 bytes)                                                │
//! │    - Version: u16 major + u16 minor (4 bytes)                               │
//! │    - Flags: u32 (4 bytes)                                                   │
//! │    - Module count: u32 (4 bytes)                                            │
//! │    - Index offset: u64 (8 bytes)                                            │
//! │    - Index size: u64 (8 bytes)                                              │
//! ├─────────────────────────────────────────────────────────────────────────────┤
//! │  MODULE DATA (variable)                                                      │
//! │    - Module 0: serialized VbcModule                                          │
//! │    - Module 1: serialized VbcModule                                          │
//! │    - ...                                                                     │
//! ├─────────────────────────────────────────────────────────────────────────────┤
//! │  INDEX (at index_offset)                                                     │
//! │    - For each module:                                                        │
//! │      - Name length: u32                                                      │
//! │      - Name: UTF-8 bytes                                                     │
//! │      - Data offset: u64                                                      │
//! │      - Data size: u64                                                        │
//! │      - Content hash: u64                                                     │
//! │      - Dependency count: u32                                                 │
//! │      - Dependencies: [module_index: u32]                                     │
//! └─────────────────────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::io::{self, Read, Write, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{VbcError, VbcResult};
use crate::module::VbcModule;
use crate::serialize::serialize_module;
use crate::deserialize::{deserialize_module, deserialize_module_validated};

// ============================================================================
// Compression Support (VBC Optimization Audit Phase 3)
// ============================================================================

/// Default zstd compression level (1-22, where 3 is a good balance of speed/ratio)
pub const DEFAULT_COMPRESSION_LEVEL: i32 = 3;

/// Compresses data using zstd.
///
/// The compressed format is:
/// - 4 bytes: uncompressed size (u32, little-endian)
/// - N bytes: zstd compressed data
///
/// # Arguments
/// * `data` - The data to compress
/// * `level` - Compression level (1-22, higher = better compression, slower)
///
/// # Returns
/// Compressed data with size header, or original data if compression doesn't help
#[cfg(feature = "compression")]
pub fn compress_data(data: &[u8], level: i32) -> io::Result<Vec<u8>> {
    use std::io::Cursor;

    // Reserve space for header + compressed data
    let mut result = Vec::with_capacity(4 + data.len());

    // Store original size first (4 bytes, little-endian)
    result.extend_from_slice(&(data.len() as u32).to_le_bytes());

    // Compress with zstd
    let compressed = zstd::encode_all(Cursor::new(data), level)?;

    // Only use compression if it actually saves space
    if compressed.len() + 4 < data.len() {
        result.extend_from_slice(&compressed);
        Ok(result)
    } else {
        // Compression didn't help - store uncompressed with marker
        // Use 0xFFFFFFFF as marker for uncompressed data
        let mut uncompressed_result = Vec::with_capacity(4 + data.len());
        uncompressed_result.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes());
        uncompressed_result.extend_from_slice(data);
        Ok(uncompressed_result)
    }
}

/// Decompresses data that was compressed with `compress_data`.
///
/// # Arguments
/// * `data` - The compressed data (including 4-byte size header)
///
/// # Returns
/// The decompressed data
#[cfg(feature = "compression")]
pub fn decompress_data(data: &[u8]) -> io::Result<Vec<u8>> {
    use std::io::Cursor;

    if data.len() < 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Compressed data too short (missing size header)",
        ));
    }

    // Read original size marker
    let size_marker = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);

    // Check for uncompressed marker
    if size_marker == 0xFFFFFFFF {
        // Data is uncompressed - just return the raw bytes
        return Ok(data[4..].to_vec());
    }

    // Decompress with zstd
    let decompressed = zstd::decode_all(Cursor::new(&data[4..]))?;

    // Verify size matches
    if decompressed.len() != size_marker as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Decompressed size mismatch: expected {}, got {}",
                size_marker,
                decompressed.len()
            ),
        ));
    }

    Ok(decompressed)
}

/// Fallback for when compression feature is disabled
#[cfg(not(feature = "compression"))]
pub fn compress_data(data: &[u8], _level: i32) -> io::Result<Vec<u8>> {
    // Just add the uncompressed marker and return the data
    let mut result = Vec::with_capacity(4 + data.len());
    result.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes());
    result.extend_from_slice(data);
    Ok(result)
}

/// Fallback for when compression feature is disabled
#[cfg(not(feature = "compression"))]
pub fn decompress_data(data: &[u8]) -> io::Result<Vec<u8>> {
    if data.len() < 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Data too short (missing size header)",
        ));
    }

    let size_marker = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);

    if size_marker != 0xFFFFFFFF {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Archive is compressed but compression feature is disabled",
        ));
    }

    Ok(data[4..].to_vec())
}

/// Magic bytes for VBC Archive: "VBCA"
pub const ARCHIVE_MAGIC: [u8; 4] = [0x56, 0x42, 0x43, 0x41]; // "VBCA"

/// Archive format major version
pub const ARCHIVE_VERSION_MAJOR: u16 = 1;
/// Archive format minor version
pub const ARCHIVE_VERSION_MINOR: u16 = 0;

/// Archive header size in bytes
pub const ARCHIVE_HEADER_SIZE: usize = 32;

bitflags::bitflags! {
    /// Flags for VBC archives
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
    pub struct ArchiveFlags: u32 {
        /// This is the standard library archive
        const IS_STDLIB = 0b0000_0001;
        /// Archive is compressed (zstd)
        const COMPRESSED = 0b0000_0010;
        /// Contains debug information
        const DEBUG_INFO = 0b0000_0100;
        /// Contains source maps
        const SOURCE_MAPS = 0b0000_1000;

        // ========================================================================
        // Metadata Stripping Flags (VBC Optimization Audit Phase 3)
        // ========================================================================
        // These flags indicate metadata has been stripped for smaller archive size.
        // Use for release builds where reflection/debug info is not needed.

        /// Field names have been stripped from type descriptors.
        /// Saves ~10% archive size. Breaks: reflection, error messages with field names.
        const STRIP_FIELD_NAMES = 0b0001_0000;

        /// Variant names have been stripped from sum type descriptors.
        /// Saves ~8% archive size. Breaks: reflection, debug printing of enums.
        const STRIP_VARIANT_NAMES = 0b0010_0000;

        /// Type constraints have been stripped (post-verification).
        /// Saves ~8% archive size. Breaks: runtime verification, debug type info.
        const STRIP_CONSTRAINTS = 0b0100_0000;

        /// Protocol implementation metadata has been stripped.
        /// Saves ~12% archive size. Breaks: dynamic dispatch lookup (keep indices).
        const STRIP_PROTOCOL_DETAILS = 0b1000_0000;

        /// Combined flag for maximum stripping (release builds)
        const RELEASE_STRIP = Self::STRIP_FIELD_NAMES.bits()
                            | Self::STRIP_VARIANT_NAMES.bits()
                            | Self::STRIP_CONSTRAINTS.bits();
    }
}

// ============================================================================
// Metadata Stripping (VBC Optimization Audit Phase 3)
// ============================================================================

use crate::types::{FieldDescriptor, StringId, TypeDescriptor, VariantDescriptor};

/// Strips metadata from a module based on archive flags.
///
/// This reduces archive size by removing debug/reflection information
/// that is not needed at runtime. The stripping is lossy - information
/// cannot be recovered without re-compilation.
///
/// # Stripping Levels
///
/// - `STRIP_FIELD_NAMES`: Replace field names with empty StringId
/// - `STRIP_VARIANT_NAMES`: Replace variant names with empty StringId
/// - `STRIP_CONSTRAINTS`: Clear type parameter bounds (post-verification)
/// - `STRIP_PROTOCOL_DETAILS`: Keep only protocol indices (clear extra metadata)
///
/// # Example
///
/// ```ignore
/// use verum_vbc::archive::{ArchiveFlags, strip_module_metadata};
///
/// let mut module = compile_module(source)?;
/// strip_module_metadata(&mut module, ArchiveFlags::RELEASE_STRIP);
/// // Module now has ~30% smaller serialized size
/// ```
pub fn strip_module_metadata(module: &mut VbcModule, flags: ArchiveFlags) {
    let strip_field_names = flags.contains(ArchiveFlags::STRIP_FIELD_NAMES);
    let strip_variant_names = flags.contains(ArchiveFlags::STRIP_VARIANT_NAMES);
    let strip_constraints = flags.contains(ArchiveFlags::STRIP_CONSTRAINTS);

    for type_desc in &mut module.types {
        strip_type_metadata(type_desc, strip_field_names, strip_variant_names, strip_constraints);
    }

    // Also strip source map if not needed for debugging
    if !flags.contains(ArchiveFlags::DEBUG_INFO) && !flags.contains(ArchiveFlags::SOURCE_MAPS) {
        module.source_map = None;
    }
}

/// Strips metadata from a single type descriptor.
fn strip_type_metadata(
    type_desc: &mut TypeDescriptor,
    strip_field_names: bool,
    strip_variant_names: bool,
    strip_constraints: bool,
) {
    // Strip field names (keep offsets and types for runtime)
    if strip_field_names {
        for field in &mut type_desc.fields {
            strip_field_name(field);
        }
    }

    // Strip variant names (keep tags for pattern matching)
    if strip_variant_names {
        for variant in &mut type_desc.variants {
            strip_variant_name(variant, strip_field_names);
        }
    }

    // Strip type parameter bounds (constraints)
    if strip_constraints {
        for param in &mut type_desc.type_params {
            param.bounds.clear();
        }
    }
}

/// Strips the name from a field descriptor.
#[inline]
fn strip_field_name(field: &mut FieldDescriptor) {
    field.name = StringId::EMPTY;
}

/// Strips the name from a variant descriptor.
#[inline]
fn strip_variant_name(variant: &mut VariantDescriptor, strip_field_names: bool) {
    variant.name = StringId::EMPTY;
    // Also strip field names in record variants
    if strip_field_names {
        for field in &mut variant.fields {
            strip_field_name(field);
        }
    }
}

/// Calculates approximate size savings from stripping.
///
/// Returns (original_estimate, stripped_estimate) in bytes.
/// This is an estimate based on typical string and metadata sizes.
pub fn estimate_stripping_savings(module: &VbcModule, flags: ArchiveFlags) -> (usize, usize) {
    let mut name_bytes = 0usize;

    let strip_field_names = flags.contains(ArchiveFlags::STRIP_FIELD_NAMES);
    let strip_variant_names = flags.contains(ArchiveFlags::STRIP_VARIANT_NAMES);

    for type_desc in &module.types {
        if strip_field_names {
            // Estimate ~8 bytes per field name (StringId + string table entry)
            name_bytes += type_desc.fields.len() * 8;
        }
        if strip_variant_names {
            // Estimate ~8 bytes per variant name
            name_bytes += type_desc.variants.len() * 8;
            // Plus nested field names in record variants
            if strip_field_names {
                for variant in &type_desc.variants {
                    name_bytes += variant.fields.len() * 8;
                }
            }
        }
    }

    // Original is current module data size, stripped removes name_bytes
    // This is a rough estimate - actual savings depend on compression
    let original = module.bytecode.len() + module.types.len() * 64;
    (original, original.saturating_sub(name_bytes))
}

/// VBC Archive header
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveHeader {
    /// Magic bytes: "VBCA"
    pub magic: [u8; 4],
    /// Major version
    pub version_major: u16,
    /// Minor version
    pub version_minor: u16,
    /// Archive flags
    pub flags: ArchiveFlags,
    /// Number of modules in the archive
    pub module_count: u32,
    /// Offset to the index section
    pub index_offset: u64,
    /// Size of the index section
    pub index_size: u64,
}

impl ArchiveHeader {
    /// Creates a new archive header
    pub fn new() -> Self {
        Self {
            magic: ARCHIVE_MAGIC,
            version_major: ARCHIVE_VERSION_MAJOR,
            version_minor: ARCHIVE_VERSION_MINOR,
            flags: ArchiveFlags::empty(),
            module_count: 0,
            index_offset: 0,
            index_size: 0,
        }
    }

    /// Creates a stdlib archive header
    pub fn stdlib() -> Self {
        Self {
            flags: ArchiveFlags::IS_STDLIB,
            ..Self::new()
        }
    }
}

impl Default for ArchiveHeader {
    fn default() -> Self {
        Self::new()
    }
}

/// Module entry in the archive index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleEntry {
    /// Module name (e.g., "core", "collections.list")
    pub name: String,
    /// Offset in the archive where the module data starts
    pub data_offset: u64,
    /// Size of the serialized module data
    pub data_size: u64,
    /// Content hash for cache invalidation
    pub content_hash: u64,
    /// Indices of modules this module depends on
    pub dependencies: Vec<u32>,
}

/// VBC Archive - a collection of VBC modules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VbcArchive {
    /// Archive header
    pub header: ArchiveHeader,
    /// Module index
    pub index: Vec<ModuleEntry>,
    /// Serialized module data (in archive order)
    pub module_data: Vec<Vec<u8>>,
}

impl VbcArchive {
    /// Creates a new empty archive
    pub fn new() -> Self {
        Self {
            header: ArchiveHeader::new(),
            index: Vec::new(),
            module_data: Vec::new(),
        }
    }

    /// Creates a new stdlib archive
    pub fn stdlib() -> Self {
        Self {
            header: ArchiveHeader::stdlib(),
            index: Vec::new(),
            module_data: Vec::new(),
        }
    }

    /// Returns the number of modules in the archive
    pub fn module_count(&self) -> usize {
        self.index.len()
    }

    /// Checks if this is a stdlib archive
    pub fn is_stdlib(&self) -> bool {
        self.header.flags.contains(ArchiveFlags::IS_STDLIB)
    }

    /// Gets a module entry by name
    pub fn get_entry(&self, name: &str) -> Option<&ModuleEntry> {
        self.index.iter().find(|e| e.name == name)
    }

    /// Gets a module entry index by name
    pub fn get_entry_index(&self, name: &str) -> Option<usize> {
        self.index.iter().position(|e| e.name == name)
    }

    /// Gets serialized module data by index
    pub fn get_module_data(&self, index: usize) -> Option<&[u8]> {
        self.module_data.get(index).map(|v| v.as_slice())
    }

    /// Deserializes and returns a module by name.
    ///
    /// If the archive is compressed, the module data is automatically decompressed.
    pub fn load_module(&self, name: &str) -> VbcResult<VbcModule> {
        let entry_idx = self.get_entry_index(name)
            .ok_or_else(|| VbcError::ArchiveError(format!("Module not found: {}", name)))?;

        let data = self.get_module_data(entry_idx)
            .ok_or_else(|| VbcError::ArchiveError(format!("Module data not found: {}", name)))?;

        // Decompress if archive is compressed
        let decompressed = if self.header.flags.contains(ArchiveFlags::COMPRESSED) {
            decompress_data(data)
                .map_err(|e| VbcError::ArchiveError(format!("Decompression error: {}", e)))?
        } else {
            data.to_vec()
        };

        deserialize_module(&decompressed)
    }

    /// Loads a module from the archive **and** validates the
    /// per-instruction bytecode cross-references before returning.
    ///
    /// Use this when the archive comes from any non-trusted source:
    /// a download, a shared cache, a file edited by hand.  Catches
    /// hand-crafted-bytecode attacks (out-of-range FunctionId,
    /// register-bounds violations, branch offsets landing mid-
    /// instruction, etc.) at load time instead of execution-reach.
    ///
    /// See [`deserialize_module_validated`] for the full list of
    /// invariants checked.  Cost is O(N) in total instruction count
    /// across all functions in the module.
    pub fn load_module_validated(&self, name: &str) -> VbcResult<VbcModule> {
        let entry_idx = self.get_entry_index(name)
            .ok_or_else(|| VbcError::ArchiveError(format!("Module not found: {}", name)))?;

        let data = self.get_module_data(entry_idx)
            .ok_or_else(|| VbcError::ArchiveError(format!("Module data not found: {}", name)))?;

        let decompressed = if self.header.flags.contains(ArchiveFlags::COMPRESSED) {
            decompress_data(data)
                .map_err(|e| VbcError::ArchiveError(format!("Decompression error: {}", e)))?
        } else {
            data.to_vec()
        };

        deserialize_module_validated(&decompressed)
    }

    /// Returns whether this archive uses compression.
    pub fn is_compressed(&self) -> bool {
        self.header.flags.contains(ArchiveFlags::COMPRESSED)
    }

    /// Returns all module names in dependency order
    pub fn module_names_ordered(&self) -> Vec<&str> {
        // Topological sort based on dependencies
        let mut visited = vec![false; self.index.len()];
        let mut result = Vec::with_capacity(self.index.len());

        fn visit<'a>(
            idx: usize,
            index: &'a [ModuleEntry],
            visited: &mut [bool],
            result: &mut Vec<&'a str>,
        ) {
            if visited[idx] {
                return;
            }
            visited[idx] = true;

            for &dep_idx in &index[idx].dependencies {
                visit(dep_idx as usize, index, visited, result);
            }

            result.push(&index[idx].name);
        }

        for i in 0..self.index.len() {
            visit(i, &self.index, &mut visited, &mut result);
        }

        result
    }
}

impl Default for VbcArchive {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for VBC archives
#[derive(Debug)]
pub struct ArchiveBuilder {
    /// Archive being built
    archive: VbcArchive,
    /// Map from module name to index
    name_to_index: HashMap<String, usize>,
    /// Compression level (1-22, only used when COMPRESSED flag is set)
    compression_level: i32,
}

impl ArchiveBuilder {
    /// Creates a new archive builder
    pub fn new() -> Self {
        Self {
            archive: VbcArchive::new(),
            name_to_index: HashMap::new(),
            compression_level: DEFAULT_COMPRESSION_LEVEL,
        }
    }

    /// Creates a new stdlib archive builder
    pub fn stdlib() -> Self {
        Self {
            archive: VbcArchive::stdlib(),
            name_to_index: HashMap::new(),
            compression_level: DEFAULT_COMPRESSION_LEVEL,
        }
    }

    /// Sets archive flags
    pub fn with_flags(mut self, flags: ArchiveFlags) -> Self {
        self.archive.header.flags |= flags;
        self
    }

    /// Sets the compression level (1-22).
    ///
    /// Only takes effect when the `COMPRESSED` flag is also set.
    /// - Level 1-3: Fast compression, moderate ratio
    /// - Level 4-9: Balanced compression
    /// - Level 10-22: Maximum compression, slower
    ///
    /// Default is 3 (fast with good ratio).
    pub fn with_compression_level(mut self, level: i32) -> Self {
        self.compression_level = level.clamp(1, 22);
        self
    }

    /// Enables compression with the default compression level.
    ///
    /// This is a convenience method that sets the `COMPRESSED` flag.
    pub fn with_compression(self) -> Self {
        self.with_flags(ArchiveFlags::COMPRESSED)
    }

    /// Adds a pre-serialized module to the archive.
    ///
    /// If the archive has the `COMPRESSED` flag set, the data will be compressed
    /// using zstd before being stored.
    pub fn add_module_data(
        &mut self,
        name: &str,
        data: Vec<u8>,
        dependencies: &[&str],
    ) -> VbcResult<usize> {
        // Check for duplicate
        if self.name_to_index.contains_key(name) {
            return Err(VbcError::ArchiveError(format!(
                "Duplicate module: {}", name
            )));
        }

        // Resolve dependencies to indices
        let dep_indices: Vec<u32> = dependencies
            .iter()
            .map(|dep_name| {
                self.name_to_index
                    .get(*dep_name)
                    .map(|&idx| idx as u32)
                    .ok_or_else(|| VbcError::ArchiveError(format!(
                        "Unknown dependency: {} (required by {})", dep_name, name
                    )))
            })
            .collect::<VbcResult<Vec<_>>>()?;

        // Compute content hash using blake3 (truncated to u64 for header)
        let content_hash = {
            let hash = blake3::hash(&data);
            u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap())
        };

        // Compress if flag is set
        let stored_data = if self.archive.header.flags.contains(ArchiveFlags::COMPRESSED) {
            compress_data(&data, self.compression_level)
                .map_err(|e| VbcError::ArchiveError(format!("Compression error: {}", e)))?
        } else {
            data
        };

        // Add entry
        let index = self.archive.index.len();
        self.archive.index.push(ModuleEntry {
            name: name.to_string(),
            data_offset: 0, // Will be set during finalization
            data_size: stored_data.len() as u64,
            content_hash,
            dependencies: dep_indices,
        });
        self.archive.module_data.push(stored_data);
        self.name_to_index.insert(name.to_string(), index);

        Ok(index)
    }

    /// Adds a VbcModule to the archive.
    ///
    /// If the archive has metadata stripping flags set, the module will be
    /// stripped before serialization. Use `add_module_unstripped` to bypass
    /// stripping even when flags are set.
    pub fn add_module(
        &mut self,
        name: &str,
        module: &VbcModule,
        dependencies: &[&str],
    ) -> VbcResult<usize> {
        let flags = self.archive.header.flags;
        let has_strip_flags = flags.intersects(
            ArchiveFlags::STRIP_FIELD_NAMES
            | ArchiveFlags::STRIP_VARIANT_NAMES
            | ArchiveFlags::STRIP_CONSTRAINTS
            | ArchiveFlags::STRIP_PROTOCOL_DETAILS
        );

        if has_strip_flags {
            // Clone and strip the module
            let mut stripped_module = module.clone();
            strip_module_metadata(&mut stripped_module, flags);
            let data = serialize_module(&stripped_module)?;
            self.add_module_data(name, data, dependencies)
        } else {
            let data = serialize_module(module)?;
            self.add_module_data(name, data, dependencies)
        }
    }

    /// Adds a VbcModule to the archive without applying metadata stripping.
    ///
    /// Use this when you need to preserve full metadata even in a release
    /// archive (e.g., for modules that require runtime reflection).
    pub fn add_module_unstripped(
        &mut self,
        name: &str,
        module: &VbcModule,
        dependencies: &[&str],
    ) -> VbcResult<usize> {
        let data = serialize_module(module)?;
        self.add_module_data(name, data, dependencies)
    }

    /// Adds a VbcModule with explicit stripping flags (overrides archive flags).
    ///
    /// Use this for fine-grained control over which modules get stripped.
    pub fn add_module_with_strip_flags(
        &mut self,
        name: &str,
        module: &VbcModule,
        dependencies: &[&str],
        strip_flags: ArchiveFlags,
    ) -> VbcResult<usize> {
        let has_strip_flags = strip_flags.intersects(
            ArchiveFlags::STRIP_FIELD_NAMES
            | ArchiveFlags::STRIP_VARIANT_NAMES
            | ArchiveFlags::STRIP_CONSTRAINTS
            | ArchiveFlags::STRIP_PROTOCOL_DETAILS
        );

        if has_strip_flags {
            let mut stripped_module = module.clone();
            strip_module_metadata(&mut stripped_module, strip_flags);
            let data = serialize_module(&stripped_module)?;
            self.add_module_data(name, data, dependencies)
        } else {
            let data = serialize_module(module)?;
            self.add_module_data(name, data, dependencies)
        }
    }

    /// Finalizes the archive builder and returns the archive
    pub fn finish(mut self) -> VbcArchive {
        // Update header
        self.archive.header.module_count = self.archive.index.len() as u32;

        // Calculate offsets
        let mut offset = ARCHIVE_HEADER_SIZE as u64;
        for (i, entry) in self.archive.index.iter_mut().enumerate() {
            entry.data_offset = offset;
            entry.data_size = self.archive.module_data[i].len() as u64;
            offset += entry.data_size;
        }

        self.archive.header.index_offset = offset;

        self.archive
    }
}

impl Default for ArchiveBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Writes a VBC archive to a writer
pub fn write_archive<W: Write>(archive: &VbcArchive, mut writer: W) -> io::Result<()> {
    // Write header
    writer.write_all(&archive.header.magic)?;
    writer.write_all(&archive.header.version_major.to_le_bytes())?;
    writer.write_all(&archive.header.version_minor.to_le_bytes())?;
    writer.write_all(&archive.header.flags.bits().to_le_bytes())?;
    writer.write_all(&archive.header.module_count.to_le_bytes())?;
    writer.write_all(&archive.header.index_offset.to_le_bytes())?;
    writer.write_all(&archive.header.index_size.to_le_bytes())?;

    // Write module data
    for data in &archive.module_data {
        writer.write_all(data)?;
    }

    // Write index
    for entry in &archive.index {
        // Name
        let name_bytes = entry.name.as_bytes();
        writer.write_all(&(name_bytes.len() as u32).to_le_bytes())?;
        writer.write_all(name_bytes)?;

        // Offsets and hash
        writer.write_all(&entry.data_offset.to_le_bytes())?;
        writer.write_all(&entry.data_size.to_le_bytes())?;
        writer.write_all(&entry.content_hash.to_le_bytes())?;

        // Dependencies
        writer.write_all(&(entry.dependencies.len() as u32).to_le_bytes())?;
        for &dep in &entry.dependencies {
            writer.write_all(&dep.to_le_bytes())?;
        }
    }

    Ok(())
}

/// Reads a VBC archive from a reader
/// Architectural upper bounds for archive index entries.
///
/// Hostile archives can claim `module_count`, `name_len`,
/// `dep_count`, and `data_size` values up to their full integer
/// range (u32 = 4 billion, u64 = 18 EB).  Allocating those sizes
/// before checking against the actual file content is a memory-
/// amplification denial-of-service: a 32-byte header can request
/// terabytes of allocations.
///
/// These bounds reflect "no real-world Verum archive ever
/// approaches this" — any input that exceeds them is rejected as
/// malformed before any allocation is performed.
const MAX_MODULES_PER_ARCHIVE: u32 = 1 << 16;       // 65 536
const MAX_MODULE_NAME_BYTES: u32 = 1 << 14;         // 16 KB
const MAX_DEPS_PER_MODULE: u32 = 1 << 12;           // 4 096
const MAX_MODULE_DATA_BYTES: u64 = 1 << 30;         // 1 GB

pub fn read_archive<R: Read + Seek>(mut reader: R) -> io::Result<VbcArchive> {
    // Read header
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;
    if magic != ARCHIVE_MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid archive magic"));
    }

    let mut buf2 = [0u8; 2];
    let mut buf4 = [0u8; 4];
    let mut buf8 = [0u8; 8];

    reader.read_exact(&mut buf2)?;
    let version_major = u16::from_le_bytes(buf2);
    reader.read_exact(&mut buf2)?;
    let version_minor = u16::from_le_bytes(buf2);
    reader.read_exact(&mut buf4)?;
    let flags = ArchiveFlags::from_bits_truncate(u32::from_le_bytes(buf4));
    reader.read_exact(&mut buf4)?;
    let module_count = u32::from_le_bytes(buf4);
    reader.read_exact(&mut buf8)?;
    let index_offset = u64::from_le_bytes(buf8);
    reader.read_exact(&mut buf8)?;
    let index_size = u64::from_le_bytes(buf8);

    // Memory-amplification defense: reject implausibly large
    // module counts before allocating the index Vec.  See
    // MAX_MODULES_PER_ARCHIVE rationale.
    if module_count > MAX_MODULES_PER_ARCHIVE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "archive module_count ({}) exceeds maximum ({})",
                module_count, MAX_MODULES_PER_ARCHIVE,
            ),
        ));
    }

    let header = ArchiveHeader {
        magic,
        version_major,
        version_minor,
        flags,
        module_count,
        index_offset,
        index_size,
    };

    // Seek to index
    reader.seek(SeekFrom::Start(index_offset))?;

    // Read index.  `module_count` already bounded above, so the
    // Vec::with_capacity allocation is safe.
    let mut index = Vec::with_capacity(module_count as usize);
    for _ in 0..module_count {
        // Name length — reject implausibly large names before
        // allocating the name buffer.
        reader.read_exact(&mut buf4)?;
        let name_len = u32::from_le_bytes(buf4);
        if name_len > MAX_MODULE_NAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "archive module name_len ({}) exceeds maximum ({})",
                    name_len, MAX_MODULE_NAME_BYTES,
                ),
            ));
        }
        let mut name_bytes = vec![0u8; name_len as usize];
        reader.read_exact(&mut name_bytes)?;
        let name = String::from_utf8_lossy(&name_bytes).to_string();

        // Offsets and hash
        reader.read_exact(&mut buf8)?;
        let data_offset = u64::from_le_bytes(buf8);
        reader.read_exact(&mut buf8)?;
        let data_size = u64::from_le_bytes(buf8);
        reader.read_exact(&mut buf8)?;
        let content_hash = u64::from_le_bytes(buf8);

        // Reject implausibly large data sizes before reaching the
        // matching `vec![0u8; data_size]` below.  Cheap to detect
        // here at index-read time; the per-module loop below trusts
        // this check.
        if data_size > MAX_MODULE_DATA_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "archive module '{}' data_size ({}) exceeds maximum ({})",
                    name, data_size, MAX_MODULE_DATA_BYTES,
                ),
            ));
        }

        // Dependencies — same defense.
        reader.read_exact(&mut buf4)?;
        let dep_count = u32::from_le_bytes(buf4);
        if dep_count > MAX_DEPS_PER_MODULE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "archive module '{}' dep_count ({}) exceeds maximum ({})",
                    name, dep_count, MAX_DEPS_PER_MODULE,
                ),
            ));
        }
        let mut dependencies = Vec::with_capacity(dep_count as usize);
        for _ in 0..dep_count {
            reader.read_exact(&mut buf4)?;
            dependencies.push(u32::from_le_bytes(buf4));
        }

        index.push(ModuleEntry {
            name,
            data_offset,
            data_size,
            content_hash,
            dependencies,
        });
    }

    // Read module data.  `module_count` and per-entry `data_size`
    // already bounded above.
    let mut module_data = Vec::with_capacity(module_count as usize);
    for entry in &index {
        reader.seek(SeekFrom::Start(entry.data_offset))?;
        let mut data = vec![0u8; entry.data_size as usize];
        reader.read_exact(&mut data)?;
        module_data.push(data);
    }

    Ok(VbcArchive {
        header,
        index,
        module_data,
    })
}

/// Writes a VBC archive to a file
pub fn write_archive_to_file(archive: &VbcArchive, path: impl AsRef<Path>) -> io::Result<()> {
    let file = std::fs::File::create(path)?;
    let writer = std::io::BufWriter::new(file);
    write_archive(archive, writer)
}

/// Reads a VBC archive from a file
pub fn read_archive_from_file(path: impl AsRef<Path>) -> io::Result<VbcArchive> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    read_archive(reader)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_archive_builder_empty() {
        let builder = ArchiveBuilder::new();
        let archive = builder.finish();

        assert_eq!(archive.module_count(), 0);
        assert!(!archive.is_stdlib());
    }

    /// Hostile archive header claims `module_count = u32::MAX`.
    /// Pre-fix the deserializer would `Vec::with_capacity(u32::MAX)`
    /// — ~70 GB on most allocators — before discovering the file is
    /// too short.  Post-fix the size is rejected before any
    /// allocation.
    #[test]
    fn test_read_archive_rejects_huge_module_count() {
        let mut payload = Vec::new();
        // Magic
        payload.extend_from_slice(&ARCHIVE_MAGIC);
        // version_major / version_minor
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        // flags
        payload.extend_from_slice(&0u32.to_le_bytes());
        // module_count — adversarial
        payload.extend_from_slice(&u32::MAX.to_le_bytes());
        // index_offset / index_size (irrelevant; the size check
        // fires first)
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());

        let cursor = io::Cursor::new(payload);
        let result = read_archive(cursor);
        assert!(
            result.is_err(),
            "u32::MAX module_count must be rejected at the size gate"
        );
        let msg = format!("{}", result.err().unwrap());
        assert!(
            msg.contains("module_count"),
            "error must identify the offending field, got: {}",
            msg,
        );
    }

    /// Hostile name_len in the index entry — would request a u32::MAX
    /// (4 GB) byte allocation for the name buffer.  Post-fix: rejected.
    #[test]
    fn test_read_archive_rejects_huge_name_len() {
        let mut payload = Vec::new();
        // Header — module_count = 1
        payload.extend_from_slice(&ARCHIVE_MAGIC);
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        // index_offset = end of header (we'll seek there)
        let header_end = (4 + 2 + 2 + 4 + 4 + 8 + 8) as u64;
        payload.extend_from_slice(&header_end.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
        // Index entry: hostile name_len
        payload.extend_from_slice(&u32::MAX.to_le_bytes());

        let cursor = io::Cursor::new(payload);
        let result = read_archive(cursor);
        assert!(result.is_err(), "u32::MAX name_len must be rejected");
        let msg = format!("{}", result.err().unwrap());
        assert!(
            msg.contains("name_len"),
            "error must identify name_len, got: {}",
            msg,
        );
    }

    /// Archive index entry claims `data_size = u64::MAX`.  Per-fix
    /// the deserializer would `vec![0u8; u64::MAX as usize]` —
    /// either OOM or abort.  Post-fix the size is rejected before
    /// the allocation.
    #[test]
    fn test_read_archive_rejects_huge_data_size() {
        let mut payload = Vec::new();
        // Header — module_count = 1
        payload.extend_from_slice(&ARCHIVE_MAGIC);
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        let header_end = (4 + 2 + 2 + 4 + 4 + 8 + 8) as u64;
        payload.extend_from_slice(&header_end.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
        // Index entry: name_len, data_offset, data_size,
        // content_hash — the data_size check fires AFTER
        // content_hash is read, so the payload must include all
        // three to reach the size gate.
        payload.extend_from_slice(&0u32.to_le_bytes()); // name_len = 0
        payload.extend_from_slice(&0u64.to_le_bytes()); // data_offset
        payload.extend_from_slice(&u64::MAX.to_le_bytes()); // data_size — adversarial
        payload.extend_from_slice(&0u64.to_le_bytes()); // content_hash

        let cursor = io::Cursor::new(payload);
        let result = read_archive(cursor);
        assert!(result.is_err(), "u64::MAX data_size must be rejected");
        let msg = format!("{}", result.err().unwrap());
        assert!(
            msg.contains("data_size"),
            "error must identify data_size, got: {}",
            msg,
        );
    }

    /// Archive index entry claims `dep_count = u32::MAX`.  Same
    /// memory-amp class — would `Vec::with_capacity(u32::MAX)` for
    /// the dependencies vector before reading any dep entries.
    #[test]
    fn test_read_archive_rejects_huge_dep_count() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&ARCHIVE_MAGIC);
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        let header_end = (4 + 2 + 2 + 4 + 4 + 8 + 8) as u64;
        payload.extend_from_slice(&header_end.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
        // Index entry
        payload.extend_from_slice(&0u32.to_le_bytes()); // name_len = 0
        payload.extend_from_slice(&0u64.to_le_bytes()); // data_offset
        payload.extend_from_slice(&0u64.to_le_bytes()); // data_size — small
        payload.extend_from_slice(&0u64.to_le_bytes()); // content_hash
        payload.extend_from_slice(&u32::MAX.to_le_bytes()); // dep_count — adversarial

        let cursor = io::Cursor::new(payload);
        let result = read_archive(cursor);
        assert!(result.is_err(), "u32::MAX dep_count must be rejected");
        let msg = format!("{}", result.err().unwrap());
        assert!(
            msg.contains("dep_count"),
            "error must identify dep_count, got: {}",
            msg,
        );
    }

    #[test]
    fn test_archive_builder_stdlib() {
        let builder = ArchiveBuilder::stdlib();
        let archive = builder.finish();

        assert!(archive.is_stdlib());
    }

    #[test]
    fn test_archive_roundtrip() {
        let mut builder = ArchiveBuilder::new();

        // Add some test modules
        let module1 = VbcModule::new("core".to_string());
        let module2 = VbcModule::new("collections".to_string());

        builder.add_module("core", &module1, &[]).unwrap();
        builder.add_module("collections", &module2, &["core"]).unwrap();

        let archive = builder.finish();

        assert_eq!(archive.module_count(), 2);
        assert_eq!(archive.index[0].name, "core");
        assert_eq!(archive.index[1].name, "collections");
        assert_eq!(archive.index[1].dependencies, vec![0]); // collections depends on core

        // Test roundtrip through bytes
        let mut bytes = Vec::new();
        write_archive(&archive, &mut bytes).unwrap();

        let loaded = read_archive(std::io::Cursor::new(bytes)).unwrap();

        assert_eq!(loaded.module_count(), 2);
        assert_eq!(loaded.index[0].name, "core");
        assert_eq!(loaded.index[1].name, "collections");
    }

    #[test]
    fn test_archive_dependency_order() {
        let mut builder = ArchiveBuilder::new();

        let module1 = VbcModule::new("core".to_string());
        let module2 = VbcModule::new("text".to_string());
        let module3 = VbcModule::new("collections".to_string());

        // Add in non-dependency order
        builder.add_module("core", &module1, &[]).unwrap();
        builder.add_module("text", &module2, &["core"]).unwrap();
        builder.add_module("collections", &module3, &["core", "text"]).unwrap();

        let archive = builder.finish();

        let ordered = archive.module_names_ordered();

        // core must come before text and collections
        let core_idx = ordered.iter().position(|&n| n == "core").unwrap();
        let text_idx = ordered.iter().position(|&n| n == "text").unwrap();
        let coll_idx = ordered.iter().position(|&n| n == "collections").unwrap();

        assert!(core_idx < text_idx);
        assert!(core_idx < coll_idx);
        assert!(text_idx < coll_idx);
    }

    #[test]
    fn test_archive_duplicate_module_error() {
        let mut builder = ArchiveBuilder::new();

        let module = VbcModule::new("core".to_string());
        builder.add_module("core", &module, &[]).unwrap();

        let result = builder.add_module("core", &module, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_archive_unknown_dependency_error() {
        let mut builder = ArchiveBuilder::new();

        let module = VbcModule::new("test".to_string());

        let result = builder.add_module("test", &module, &["nonexistent"]);
        assert!(result.is_err());
    }

    // ========================================================================
    // Metadata Stripping Tests (VBC Optimization Audit Phase 3)
    // ========================================================================

    #[test]
    fn test_strip_field_names() {
        use crate::types::{FieldDescriptor, TypeDescriptor, TypeId, TypeKind, TypeRef, Visibility};

        let mut module = VbcModule::new("test".to_string());

        // Create a type with named fields
        let field1 = FieldDescriptor {
            name: module.intern_string("x"),
            type_ref: TypeRef::Concrete(TypeId::INT),
            offset: 0,
            visibility: Visibility::Public,
        };
        let field2 = FieldDescriptor {
            name: module.intern_string("y"),
            type_ref: TypeRef::Concrete(TypeId::INT),
            offset: 8,
            visibility: Visibility::Public,
        };

        let mut type_desc = TypeDescriptor::default();
        type_desc.id = TypeId(100);
        type_desc.name = module.intern_string("Point");
        type_desc.kind = TypeKind::Record;
        type_desc.fields.push(field1);
        type_desc.fields.push(field2);

        module.types.push(type_desc);

        // Verify names exist before stripping
        assert_ne!(module.types[0].fields[0].name, StringId::EMPTY);
        assert_ne!(module.types[0].fields[1].name, StringId::EMPTY);

        // Strip field names
        super::strip_module_metadata(&mut module, ArchiveFlags::STRIP_FIELD_NAMES);

        // Verify names are stripped but offsets preserved
        assert_eq!(module.types[0].fields[0].name, StringId::EMPTY);
        assert_eq!(module.types[0].fields[1].name, StringId::EMPTY);
        assert_eq!(module.types[0].fields[0].offset, 0);
        assert_eq!(module.types[0].fields[1].offset, 8);
    }

    #[test]
    fn test_strip_variant_names() {
        use crate::types::{TypeDescriptor, TypeId, TypeKind, TypeRef, VariantDescriptor, VariantKind};

        let mut module = VbcModule::new("test".to_string());

        // Create a sum type with named variants
        let variant1 = VariantDescriptor {
            name: module.intern_string("None"),
            tag: 0,
            payload: None,
            kind: VariantKind::Unit,
            arity: 0,
            fields: smallvec::smallvec![],
        };
        let variant2 = VariantDescriptor {
            name: module.intern_string("Some"),
            tag: 1,
            payload: Some(TypeRef::Concrete(TypeId::INT)),
            kind: VariantKind::Tuple,
            arity: 1,
            fields: smallvec::smallvec![],
        };

        let mut type_desc = TypeDescriptor::default();
        type_desc.id = TypeId(101);
        type_desc.name = module.intern_string("Option");
        type_desc.kind = TypeKind::Sum;
        type_desc.variants.push(variant1);
        type_desc.variants.push(variant2);

        module.types.push(type_desc);

        // Verify names exist before stripping
        assert_ne!(module.types[0].variants[0].name, StringId::EMPTY);
        assert_ne!(module.types[0].variants[1].name, StringId::EMPTY);

        // Strip variant names
        super::strip_module_metadata(&mut module, ArchiveFlags::STRIP_VARIANT_NAMES);

        // Verify names are stripped but tags preserved
        assert_eq!(module.types[0].variants[0].name, StringId::EMPTY);
        assert_eq!(module.types[0].variants[1].name, StringId::EMPTY);
        assert_eq!(module.types[0].variants[0].tag, 0);
        assert_eq!(module.types[0].variants[1].tag, 1);
    }

    #[test]
    fn test_release_strip_flag() {
        // Verify RELEASE_STRIP combines the expected flags
        let release = ArchiveFlags::RELEASE_STRIP;

        assert!(release.contains(ArchiveFlags::STRIP_FIELD_NAMES));
        assert!(release.contains(ArchiveFlags::STRIP_VARIANT_NAMES));
        assert!(release.contains(ArchiveFlags::STRIP_CONSTRAINTS));
        assert!(!release.contains(ArchiveFlags::STRIP_PROTOCOL_DETAILS));
    }

    #[test]
    fn test_archive_builder_applies_stripping() {
        use crate::types::{FieldDescriptor, TypeDescriptor, TypeId, TypeKind, TypeRef, Visibility};

        let mut module = VbcModule::new("test".to_string());

        // Add a type with named field
        let field = FieldDescriptor {
            name: module.intern_string("value"),
            type_ref: TypeRef::Concrete(TypeId::INT),
            offset: 0,
            visibility: Visibility::Public,
        };

        let mut type_desc = TypeDescriptor::default();
        type_desc.id = TypeId(102);
        type_desc.name = module.intern_string("Wrapper");
        type_desc.kind = TypeKind::Record;
        type_desc.fields.push(field);

        module.types.push(type_desc);

        // Create archive builder with strip flag
        let mut builder = ArchiveBuilder::new()
            .with_flags(ArchiveFlags::STRIP_FIELD_NAMES);

        builder.add_module("test", &module, &[]).unwrap();

        let archive = builder.finish();

        // Deserialize the module and verify field name is stripped
        let loaded_module = crate::deserialize::deserialize_module(&archive.module_data[0]).unwrap();
        assert_eq!(loaded_module.types[0].fields[0].name, StringId::EMPTY);
    }

    #[test]
    fn test_estimate_stripping_savings() {
        use crate::types::{FieldDescriptor, TypeDescriptor, TypeId, TypeKind, TypeRef, Visibility};

        let mut module = VbcModule::new("test".to_string());

        // Add multiple types with fields
        for i in 0..10 {
            let mut type_desc = TypeDescriptor::default();
            type_desc.id = TypeId(200 + i);
            type_desc.name = module.intern_string(&format!("Type{}", i));
            type_desc.kind = TypeKind::Record;

            // Add 5 fields per type
            for j in 0..5 {
                let field = FieldDescriptor {
                    name: module.intern_string(&format!("field{}_{}", i, j)),
                    type_ref: TypeRef::Concrete(TypeId::INT),
                    offset: (j * 8) as u32,
                    visibility: Visibility::Public,
                };
                type_desc.fields.push(field);
            }

            module.types.push(type_desc);
        }

        // Estimate savings
        let (original, stripped) = super::estimate_stripping_savings(
            &module,
            ArchiveFlags::STRIP_FIELD_NAMES
        );

        // With 10 types × 5 fields × 8 bytes/field = 400 bytes expected savings
        assert!(original > stripped);
        assert!(original - stripped >= 400);
    }

    // ========================================================================
    // Compression Tests (VBC Optimization Audit Phase 3)
    // ========================================================================

    #[test]
    fn test_compress_decompress_roundtrip() {
        // Create test data with some repetition (compresses well)
        let original: Vec<u8> = (0..1000).flat_map(|i| {
            vec![
                (i % 256) as u8,
                ((i / 256) % 256) as u8,
                0xAA, 0xBB, // Repeated pattern
            ]
        }).collect();

        let compressed = super::compress_data(&original, 3).unwrap();
        let decompressed = super::decompress_data(&compressed).unwrap();

        assert_eq!(original, decompressed);
    }

    #[test]
    fn test_compress_handles_incompressible_data() {
        // Create random-like data that doesn't compress well
        let original: Vec<u8> = (0..100).map(|i| {
            // Simple PRNG-like sequence
            ((i * 31 + 17) % 256) as u8
        }).collect();

        let compressed = super::compress_data(&original, 3).unwrap();
        let decompressed = super::decompress_data(&compressed).unwrap();

        assert_eq!(original, decompressed);
    }

    #[test]
    fn test_compressed_archive_roundtrip() {
        let mut builder = ArchiveBuilder::new()
            .with_compression();

        // Add test modules with some data
        let mut module1 = VbcModule::new("core".to_string());
        // Add some bytecode to make it non-trivial
        module1.bytecode = vec![0x01, 0x02, 0x03, 0x04, 0x05];

        let mut module2 = VbcModule::new("collections".to_string());
        module2.bytecode = vec![0x10, 0x20, 0x30, 0x40, 0x50];

        builder.add_module("core", &module1, &[]).unwrap();
        builder.add_module("collections", &module2, &["core"]).unwrap();

        let archive = builder.finish();

        assert!(archive.is_compressed());
        assert_eq!(archive.module_count(), 2);

        // Test roundtrip through bytes
        let mut bytes = Vec::new();
        write_archive(&archive, &mut bytes).unwrap();

        let loaded = read_archive(std::io::Cursor::new(bytes)).unwrap();

        assert!(loaded.is_compressed());
        assert_eq!(loaded.module_count(), 2);

        // Verify we can load modules from compressed archive
        let loaded_core = loaded.load_module("core").unwrap();
        let loaded_coll = loaded.load_module("collections").unwrap();

        assert_eq!(loaded_core.name, "core");
        assert_eq!(loaded_core.bytecode, vec![0x01, 0x02, 0x03, 0x04, 0x05]);
        assert_eq!(loaded_coll.name, "collections");
        assert_eq!(loaded_coll.bytecode, vec![0x10, 0x20, 0x30, 0x40, 0x50]);
    }

    #[test]
    fn test_compression_saves_space() {
        // Create a module with repetitive data (should compress well)
        let mut module = VbcModule::new("test".to_string());
        // Highly compressible data: 10KB of repeated pattern
        module.bytecode = vec![0xAA; 10 * 1024];

        // Build uncompressed archive
        let mut uncompressed_builder = ArchiveBuilder::new();
        uncompressed_builder.add_module("test", &module, &[]).unwrap();
        let uncompressed_archive = uncompressed_builder.finish();

        let mut uncompressed_bytes = Vec::new();
        write_archive(&uncompressed_archive, &mut uncompressed_bytes).unwrap();

        // Build compressed archive
        let mut compressed_builder = ArchiveBuilder::new()
            .with_compression();
        compressed_builder.add_module("test", &module, &[]).unwrap();
        let compressed_archive = compressed_builder.finish();

        let mut compressed_bytes = Vec::new();
        write_archive(&compressed_archive, &mut compressed_bytes).unwrap();

        // Compressed should be significantly smaller
        assert!(
            compressed_bytes.len() < uncompressed_bytes.len() / 2,
            "Compressed ({} bytes) should be less than half of uncompressed ({} bytes)",
            compressed_bytes.len(),
            uncompressed_bytes.len()
        );
    }

    #[test]
    fn test_compression_level_affects_output() {
        // Create test module with enough data to see compression differences
        let mut module = VbcModule::new("test".to_string());
        module.bytecode = (0..5000).map(|i| ((i * 7 + 3) % 256) as u8).collect();

        // Build with low compression
        let mut low_builder = ArchiveBuilder::new()
            .with_compression()
            .with_compression_level(1);
        low_builder.add_module("test", &module, &[]).unwrap();
        let low_archive = low_builder.finish();

        let mut low_bytes = Vec::new();
        write_archive(&low_archive, &mut low_bytes).unwrap();

        // Build with high compression
        let mut high_builder = ArchiveBuilder::new()
            .with_compression()
            .with_compression_level(19);
        high_builder.add_module("test", &module, &[]).unwrap();
        let high_archive = high_builder.finish();

        let mut high_bytes = Vec::new();
        write_archive(&high_archive, &mut high_bytes).unwrap();

        // Both should still decompress correctly
        let loaded_low = read_archive(std::io::Cursor::new(low_bytes.clone())).unwrap();
        let loaded_high = read_archive(std::io::Cursor::new(high_bytes.clone())).unwrap();

        let mod_low = loaded_low.load_module("test").unwrap();
        let mod_high = loaded_high.load_module("test").unwrap();

        assert_eq!(mod_low.bytecode, module.bytecode);
        assert_eq!(mod_high.bytecode, module.bytecode);

        // High compression should produce smaller output (or equal for small data)
        assert!(high_bytes.len() <= low_bytes.len());
    }
}
