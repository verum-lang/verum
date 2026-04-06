//! VBC binary format definitions.
//!
//! This module defines the binary format for VBC files, including:
//! - Magic number and version
//! - Header structure (64 bytes)
//! - Section layout
//! - Compression support

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

/// Magic number for VBC files: "VBC1" in ASCII.
pub const MAGIC: [u8; 4] = [0x56, 0x42, 0x43, 0x31]; // "VBC1"

/// Current major version of VBC format.
/// Version 2: Opcode reorganization - breaks compatibility with v1 bytecode.
pub const VERSION_MAJOR: u16 = 2;

/// Current minor version of VBC format.
pub const VERSION_MINOR: u16 = 0;

/// Size of VBC header in bytes.
/// 4 (magic) + 2 + 2 (version) + 4 (flags) + 4 (name) +
/// 4*2 (type) + 4*2 (func) + 4*2 (const) + 4*2 (string) +
/// 4*2 (bytecode) + 4*2 (spec) + 4*2 (sourcemap) +
/// 8 (content_hash) + 8 (dep_hash) + 8 (reserved) = 96
pub const HEADER_SIZE: usize = 96;

bitflags! {
    /// VBC module flags indicating module capabilities and requirements.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct VbcFlags: u32 {
        /// Module contains generic functions.
        const HAS_GENERICS = 0b0000_0000_0001;
        /// Module contains precompiled specializations.
        const HAS_PRECOMPILED_SPECS = 0b0000_0000_0010;
        /// Module requires CBGR runtime for memory safety.
        const NEEDS_CBGR = 0b0000_0000_0100;
        /// Module contains async functions.
        const HAS_ASYNC = 0b0000_0000_1000;
        /// Module uses the context system (dependency injection).
        const HAS_CONTEXTS = 0b0000_0001_0000;
        /// Module has refinement type checks.
        const HAS_REFINEMENTS = 0b0000_0010_0000;
        /// This is a standard library module.
        const IS_STDLIB = 0b0000_0100_0000;
        /// Module contains debug information.
        const DEBUG_INFO = 0b0000_1000_0000;
        /// Sections are compressed.
        const COMPRESSED = 0b0001_0000_0000;
        /// Module contains tensor operations.
        const HAS_TENSORS = 0b0010_0000_0000;
        /// Module contains autodiff operations.
        const HAS_AUTODIFF = 0b0100_0000_0000;
        /// Module contains GPU operations.
        const HAS_GPU = 0b1000_0000_0000;
        /// Module uses FFI (foreign function interface).
        const HAS_FFI = 0b1_0000_0000_0000;

        // ======================================================================
        // Profile Flags (V-LLSI Architecture)
        // ======================================================================

        /// Module is NOT interpretable by VBC - AOT compilation required.
        ///
        /// This flag is set for modules compiled with the Systems profile.
        /// VBC serves only as an intermediate representation for these modules.
        ///
        /// Systems profile code is NOT interpretable because:
        /// - May use raw pointers and unsafe operations
        /// - May require direct hardware access
        /// - Intended for embedded/OS kernel development
        /// - VBC is portable IR only, not execution format
        ///
        /// V-LLSI flag: Module uses features incompatible with Tier 0 interpreter
        /// (inline assembly, direct syscalls, custom linker sections). Must be AOT compiled.
        const NOT_INTERPRETABLE = 0b10_0000_0000_0000;

        /// Module was compiled with Systems profile.
        ///
        /// Systems profile enables:
        /// - Raw pointers and unsafe code
        /// - Inline assembly
        /// - No libc linking (direct syscalls)
        /// - NOT VBC-interpretable (AOT only)
        const SYSTEMS_PROFILE = 0b100_0000_0000_0000;

        /// Module is for embedded/bare-metal targets.
        ///
        /// Embedded modules have additional restrictions:
        /// - No heap allocation
        /// - No OS dependencies
        /// - No async runtime
        /// - Static CBGR only
        const EMBEDDED_TARGET = 0b1000_0000_0000_0000;
    }
}

impl VbcFlags {
    /// Check if this module can be interpreted by VBC.
    ///
    /// Modules with `NOT_INTERPRETABLE` flag cannot be executed by the
    /// VBC interpreter. They must be compiled to native code via AOT.
    pub fn is_interpretable(&self) -> bool {
        !self.contains(VbcFlags::NOT_INTERPRETABLE)
    }

    /// Check if this module was compiled with Systems profile.
    pub fn is_systems_profile(&self) -> bool {
        self.contains(VbcFlags::SYSTEMS_PROFILE)
    }

    /// Check if this module targets embedded/bare-metal.
    pub fn is_embedded(&self) -> bool {
        self.contains(VbcFlags::EMBEDDED_TARGET)
    }
}

/// VBC file header (64 bytes).
///
/// The header contains all metadata needed to parse the VBC file,
/// including section offsets, sizes, and validation hashes.
///
/// # Layout
///
/// | Offset | Size | Field |
/// |--------|------|-------|
/// | 0x00 | 4 | magic: "VBC1" |
/// | 0x04 | 2 | version_major |
/// | 0x06 | 2 | version_minor |
/// | 0x08 | 4 | flags |
/// | 0x0C | 4 | module_name_offset |
/// | 0x10 | 4 | type_table_offset |
/// | 0x14 | 4 | type_table_count |
/// | 0x18 | 4 | function_table_offset |
/// | 0x1C | 4 | function_table_count |
/// | 0x20 | 4 | constant_pool_offset |
/// | 0x24 | 4 | constant_pool_count |
/// | 0x28 | 4 | string_table_offset |
/// | 0x2C | 4 | string_table_size |
/// | 0x30 | 4 | bytecode_offset |
/// | 0x34 | 4 | bytecode_size |
/// | 0x38 | 4 | specialization_table_offset |
/// | 0x3C | 4 | specialization_table_count |
/// | 0x40 | 4 | source_map_offset |
/// | 0x44 | 4 | source_map_size |
/// | 0x48 | 8 | content_hash |
/// | 0x50 | 8 | dependency_hash |
/// | 0x58 | 8 | reserved |
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VbcHeader {
    /// Magic number (must be "VBC1").
    pub magic: [u8; 4],
    /// Major version number.
    pub version_major: u16,
    /// Minor version number.
    pub version_minor: u16,
    /// Module flags.
    pub flags: VbcFlags,
    /// Offset to module name in string table.
    pub module_name_offset: u32,
    /// Offset to type table section.
    pub type_table_offset: u32,
    /// Number of entries in type table.
    pub type_table_count: u32,
    /// Offset to function table section.
    pub function_table_offset: u32,
    /// Number of entries in function table.
    pub function_table_count: u32,
    /// Offset to constant pool section.
    pub constant_pool_offset: u32,
    /// Number of entries in constant pool.
    pub constant_pool_count: u32,
    /// Offset to string table section.
    pub string_table_offset: u32,
    /// Size of string table in bytes.
    pub string_table_size: u32,
    /// Offset to bytecode section.
    pub bytecode_offset: u32,
    /// Size of bytecode section in bytes.
    pub bytecode_size: u32,
    /// Offset to specialization table section.
    pub specialization_table_offset: u32,
    /// Number of entries in specialization table.
    pub specialization_table_count: u32,
    /// Offset to source map section (0 if absent).
    pub source_map_offset: u32,
    /// Size of source map section in bytes.
    pub source_map_size: u32,
    /// XXHash64 of module content (for integrity).
    pub content_hash: u64,
    /// Hash of dependencies (for invalidation).
    pub dependency_hash: u64,
    /// Offset to extensions section (0 if absent).
    /// This section contains tensor metadata and other optional data.
    pub extensions_offset: u32,
    /// Size of extensions section in bytes.
    pub extensions_size: u32,
}

impl Default for VbcHeader {
    fn default() -> Self {
        Self::new()
    }
}

impl VbcHeader {
    /// Creates a new header with default values.
    pub fn new() -> Self {
        Self {
            magic: MAGIC,
            version_major: VERSION_MAJOR,
            version_minor: VERSION_MINOR,
            flags: VbcFlags::empty(),
            module_name_offset: 0,
            type_table_offset: HEADER_SIZE as u32,
            type_table_count: 0,
            function_table_offset: HEADER_SIZE as u32,
            function_table_count: 0,
            constant_pool_offset: HEADER_SIZE as u32,
            constant_pool_count: 0,
            string_table_offset: HEADER_SIZE as u32,
            string_table_size: 0,
            bytecode_offset: HEADER_SIZE as u32,
            bytecode_size: 0,
            specialization_table_offset: 0,
            specialization_table_count: 0,
            source_map_offset: 0,
            source_map_size: 0,
            content_hash: 0,
            dependency_hash: 0,
            extensions_offset: 0,
            extensions_size: 0,
        }
    }

    /// Checks if the magic number is valid.
    pub fn is_magic_valid(&self) -> bool {
        self.magic == MAGIC
    }

    /// Checks if the version is compatible.
    ///
    /// A VBC file is compatible if it has the same major version and
    /// a minor version less than or equal to the current minor version.
    #[allow(clippy::absurd_extreme_comparisons)]
    pub fn is_version_compatible(&self) -> bool {
        self.version_major == VERSION_MAJOR && self.version_minor <= VERSION_MINOR
    }

    /// Returns the total file size based on section offsets and sizes.
    pub fn computed_file_size(&self) -> u64 {
        let mut max = HEADER_SIZE as u64;

        // Check each section
        let sections = [
            (self.type_table_offset as u64, 0), // Size computed differently
            (
                self.function_table_offset as u64,
                0, // Size computed differently
            ),
            (
                self.constant_pool_offset as u64,
                0, // Size computed differently
            ),
            (
                self.string_table_offset as u64,
                self.string_table_size as u64,
            ),
            (self.bytecode_offset as u64, self.bytecode_size as u64),
            (
                self.source_map_offset as u64,
                self.source_map_size as u64,
            ),
        ];

        for (offset, size) in sections {
            if offset > 0 {
                let end = offset.saturating_add(size);
                max = max.max(end);
            }
        }

        max
    }
}

/// Compression algorithm for VBC sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum CompressionAlgorithm {
    /// No compression.
    None = 0,
    /// Zstandard compression (good ratio + fast decompression).
    Zstd = 1,
    /// LZ4 compression (faster decompression, worse ratio).
    Lz4 = 2,
}

impl TryFrom<u8> for CompressionAlgorithm {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(CompressionAlgorithm::None),
            1 => Ok(CompressionAlgorithm::Zstd),
            2 => Ok(CompressionAlgorithm::Lz4),
            _ => Err(value),
        }
    }
}

/// Header for a compressed section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompressedSectionHeader {
    /// Uncompressed size in bytes.
    pub uncompressed_size: u32,
    /// Compression algorithm used.
    pub algorithm: CompressionAlgorithm,
}

/// Section identifier for validation and debugging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// String table section.
    StringTable,
    /// Type table section.
    TypeTable,
    /// Function table section.
    FunctionTable,
    /// Constant pool section.
    ConstantPool,
    /// Bytecode section.
    Bytecode,
    /// Specialization table section.
    SpecializationTable,
    /// Source map section.
    SourceMap,
}

impl Section {
    /// Returns the section name for error messages.
    pub fn name(&self) -> &'static str {
        match self {
            Section::StringTable => "string_table",
            Section::TypeTable => "type_table",
            Section::FunctionTable => "function_table",
            Section::ConstantPool => "constant_pool",
            Section::Bytecode => "bytecode",
            Section::SpecializationTable => "specialization_table",
            Section::SourceMap => "source_map",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_size() {
        // Verify header size is exactly 96 bytes
        assert_eq!(HEADER_SIZE, 96);
    }

    #[test]
    fn test_magic() {
        assert_eq!(MAGIC, [0x56, 0x42, 0x43, 0x31]);
        assert_eq!(std::str::from_utf8(&MAGIC).unwrap(), "VBC1");
    }

    #[test]
    fn test_default_header() {
        let header = VbcHeader::new();
        assert!(header.is_magic_valid());
        assert!(header.is_version_compatible());
        assert_eq!(header.flags, VbcFlags::empty());
    }

    #[test]
    fn test_flags() {
        let flags = VbcFlags::HAS_GENERICS | VbcFlags::HAS_ASYNC | VbcFlags::NEEDS_CBGR;
        assert!(flags.contains(VbcFlags::HAS_GENERICS));
        assert!(flags.contains(VbcFlags::HAS_ASYNC));
        assert!(flags.contains(VbcFlags::NEEDS_CBGR));
        assert!(!flags.contains(VbcFlags::IS_STDLIB));
    }

    #[test]
    fn test_compression_algorithm() {
        assert_eq!(CompressionAlgorithm::try_from(0), Ok(CompressionAlgorithm::None));
        assert_eq!(CompressionAlgorithm::try_from(1), Ok(CompressionAlgorithm::Zstd));
        assert_eq!(CompressionAlgorithm::try_from(2), Ok(CompressionAlgorithm::Lz4));
        assert_eq!(CompressionAlgorithm::try_from(3), Err(3));
    }
}
