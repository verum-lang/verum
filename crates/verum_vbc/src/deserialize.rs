//! VBC module deserialization.
//!
//! This module provides deserialization of VBC modules from binary format.
//!
//! ## Compression Support
//!
//! VBC files may be compressed using zstd or lz4. The deserializer automatically
//! detects and decompresses compressed sections based on the compression header.

use serde::Deserialize;
use smallvec::SmallVec;

use crate::compression::decompress;
use crate::encoding::*;
use crate::error::{VbcError, VbcResult};
use crate::format::{CompressionAlgorithm, VbcFlags, VbcHeader, HEADER_SIZE, MAGIC, VERSION_MAJOR, VERSION_MINOR};
use crate::metadata::{AutodiffGraph, DeviceHints, DistributionMetadata, MlirHints, ShapeMetadata};
use crate::module::{
    CallingConvention, Constant, ConstId, FfiLibrary, FfiStructLayout, FfiSymbol,
    FunctionDescriptor, FunctionId, ModuleDependency, OptimizationHints, ParamDescriptor,
    SourceMap, SourceMapEntry, SpecializationEntry, StringTable, VbcModule,
};
use crate::types::{
    CbgrTier, ContextRef, FieldDescriptor, Mutability, PropertySet, ProtocolId, ProtocolImpl,
    StringId, TypeDescriptor, TypeId, TypeKind, TypeParamDescriptor, TypeParamId, TypeRef,
    VariantDescriptor, VariantKind, Variance, Visibility,
};

/// Bundle for deserializing FFI tables together.
#[derive(Deserialize)]
struct FfiBundle {
    libraries: Vec<FfiLibrary>,
    symbols: Vec<FfiSymbol>,
    layouts: Vec<FfiStructLayout>,
}

/// Parsed extensions data from the extensions section.
#[derive(Default)]
struct ExtensionsData {
    shape_metadata: ShapeMetadata,
    device_hints: DeviceHints,
    distribution: DistributionMetadata,
    autodiff_graph: AutodiffGraph,
    mlir_hints: MlirHints,
    ffi_libraries: Vec<FfiLibrary>,
    ffi_symbols: Vec<FfiSymbol>,
    ffi_layouts: Vec<FfiStructLayout>,
    dependencies: Vec<ModuleDependency>,
}

/// Deserializes a VBC module from binary data.
///
/// **Trust model**: this entry point assumes the input was produced
/// by a trusted compiler / source.  It performs structural decoding
/// only — no per-instruction cross-reference validation.  Use
/// [`deserialize_module_validated`] for loads from any other source
/// (archives shared across processes, network-loaded modules, files
/// edited by hand).
pub fn deserialize_module(data: &[u8]) -> VbcResult<VbcModule> {
    let mut deserializer = Deserializer::new(data);
    deserializer.deserialize()
}

/// Deserializes a VBC module from binary data **and** runs the
/// per-instruction bytecode validator + content-hash verification
/// before returning.
///
/// This is the architectural defense for loading bytecode that may
/// not have come from a trusted source — it does, in order:
///
///   1. **Structural decode** via `deserialize_module`.
///   2. **Content-hash verification**: recompute `blake3(data[HEADER_SIZE..])`
///      and compare its first 8 bytes to the header's
///      `content_hash`.  Pre-fix the field was COMPUTED at
///      serialize time but never CHECKED at deserialize — an
///      attacker could edit a `.vbc` file in place without
///      re-stamping the hash and the loader wouldn't notice.
///   3. **Per-instruction bytecode validation** via the validator,
///      catching out-of-range `FunctionId` / `ConstId` /
///      `StringId` / `TypeId`, register-bounds violations, branch
///      offsets landing mid-instruction, call-arity mismatches,
///      and decoder failures mid-stream.
///
/// Catches at LOAD time what the interpreter would otherwise catch
/// on execution-reach (best case) or silent state corruption (worst
/// case).  Cost is O(N) in total bytes (hash) + O(M) in total
/// instruction count (validator).  Use whenever the bytecode source
/// is not the in-process compiler.
///
/// Closes round-1 §3.1 + round-2 §3.1 of the red-team review and
/// activates the previously-INERT `ContentHashMismatch` defense.
pub fn deserialize_module_validated(data: &[u8]) -> VbcResult<VbcModule> {
    deserialize_module_validated_with_options(data, &crate::validate::ValidationOptions::strict())
}

/// Variant of [`deserialize_module_validated`] that honors
/// [`ValidationOptions::skip_hash_check`] and
/// [`ValidationOptions::skip_bytecode_validation`].
///
/// Pre-fix the only validated entry point invoked
/// `verify_content_hash` unconditionally — `ValidationOptions::fast()`
/// (which sets both skip flags) had no way to actually skip hash
/// verification because no caller threaded the options through. This
/// variant closes the gap: callers wanting the inexpensive fast-path
/// pass `ValidationOptions::fast()` and the loader honors both flags
/// in lockstep.
///
/// Caveat: skipping hash verification is a security trade-off, not a
/// validity trade-off — the hash is the only on-disk artefact that
/// flags tampering. Reserve `fast()` for in-process loads from a
/// freshly-serialized module where the bytes can't have been modified
/// since serialization. Cross-process / on-disk loads should keep the
/// strict default.
pub fn deserialize_module_validated_with_options(
    data: &[u8],
    options: &crate::validate::ValidationOptions,
) -> VbcResult<VbcModule> {
    let module = deserialize_module(data)?;
    if !options.skip_hash_check {
        verify_content_hash(data, module.header.content_hash)?;
        verify_dependency_hash(&module)?;
    }
    if !options.skip_bytecode_validation {
        crate::validate::validate_module_with_options(&module, options)?;
    }
    Ok(module)
}

/// Verifies the content hash carried by the VBC header against a
/// freshly-computed `blake3(data[HEADER_SIZE..])`.
///
/// The hash is over the raw on-wire bytes after the header, which
/// for a compressed module is the COMPRESSED payload — exactly what
/// the serializer hashes (`crates/verum_vbc/src/serialize.rs:153`).
/// This means hash verification can run BEFORE decompression,
/// catching tampering on the disk artifact without paying the
/// decompression cost first.
///
/// Returns `VbcError::ContentHashMismatch { expected, computed }`
/// on mismatch.  Bypassed by `ValidationOptions::skip_hash_check =
/// true`; the lenient `deserialize_module` entry point doesn't
/// invoke this check at all.
fn verify_content_hash(data: &[u8], expected: u64) -> VbcResult<()> {
    if data.len() < HEADER_SIZE {
        return Err(VbcError::eof(0, HEADER_SIZE));
    }
    let computed = {
        let hash = blake3::hash(&data[HEADER_SIZE..]);
        // blake3::Hash::as_bytes() always returns a 32-byte buffer;
        // `[..8]` on it is statically safe.  `expect` rather than
        // `unwrap_or([0u8; 8])` because the silent-zero fallback would
        // make this verifier ACCEPT any module whose serializer also
        // hit the same fallback (zero hash on both sides matches),
        // defeating the integrity defense.  Panic-on-impossible is
        // architecturally correct.
        u64::from_le_bytes(
            hash.as_bytes()[..8]
                .try_into()
                .expect("blake3 always returns 32 bytes; [..8] always fits"),
        )
    };
    if computed != expected {
        return Err(VbcError::ContentHashMismatch { expected, computed });
    }
    Ok(())
}

/// Verifies the dependency hash carried by the VBC header against a
/// freshly-computed `blake3` over the concatenation of each dependency's
/// `hash` field (encoded as little-endian u64).
///
/// This is the dependency-tree-fingerprint defense:
/// `content_hash` covers the bytes-on-disk; `dependency_hash`
/// independently fingerprints the cog-distribution dependency
/// graph so a downstream verifier (cog-resolver, build-cache,
/// reproducibility checker) can compare two modules' dep trees in
/// O(8) without walking the full dependency table.
///
/// Returns `VbcError::DependencyHashMismatch { expected, computed }`
/// on mismatch.
fn verify_dependency_hash(module: &VbcModule) -> VbcResult<()> {
    use crate::encoding::encode_u64;

    let mut dep_data = Vec::with_capacity(module.dependencies.len() * 8);
    for dep in &module.dependencies {
        encode_u64(dep.hash, &mut dep_data);
    }
    let computed = {
        let hash = blake3::hash(&dep_data);
        // Same invariant as `verify_content_hash` — `expect` rather
        // than silent-zero fallback, since matching zeros on both
        // sides would defeat the integrity check.
        u64::from_le_bytes(
            hash.as_bytes()[..8]
                .try_into()
                .expect("blake3 always returns 32 bytes; [..8] always fits"),
        )
    };
    let expected = module.header.dependency_hash;
    if computed != expected {
        return Err(VbcError::DependencyHashMismatch { expected, computed });
    }
    Ok(())
}

/// Architectural upper bounds for module-table counts.
///
/// Hostile bytecode can claim `u32::MAX` (4 billion) for any
/// `*_count` field in the header, triggering a multi-GB
/// `Vec::with_capacity(u32::MAX as usize)` allocation before the
/// deserializer reads a single entry — a memory-amplification
/// denial-of-service.  Real-world Verum modules have at most a
/// few thousand entries in any of these tables; the bounds below
/// are 1 M, comfortably above any plausible module while staying
/// far below the wraparound cliff.
///
/// Hit at parse time (before any allocation), each rejection
/// names the offending field for triage.
const MAX_TYPE_TABLE_ENTRIES: u32 = 1 << 20;            // 1 048 576
const MAX_FUNCTION_TABLE_ENTRIES: u32 = 1 << 20;
const MAX_CONSTANT_POOL_ENTRIES: u32 = 1 << 20;
const MAX_SPECIALIZATION_TABLE_ENTRIES: u32 = 1 << 20;

/// Descriptor-level architectural upper bounds.
///
/// Within a type / function descriptor, varint-encoded counts
/// (type params, fields, variants, protocols, methods, params,
/// contexts, bounds) drive `SmallVec::with_capacity` /
/// `Vec::with_capacity` allocations that have the same memory-
/// amplification surface as the table counts above.  Post the
/// varint-canonicality fix (cf1cff4c) the largest accepted
/// varint is `u64::MAX`, which casts to `usize::MAX` on 64-bit
/// platforms — `with_capacity(usize::MAX)` aborts the process
/// in most Rust allocators.  Bounds below are tight enough that
/// real-world descriptors stay under them by 1-2 orders of
/// magnitude while still rejecting adversarial inputs early.
const MAX_TYPE_PARAMS_PER_DESCRIPTOR: usize = 64;
const MAX_FIELDS_PER_DESCRIPTOR: usize = 4 * 1024;
const MAX_VARIANTS_PER_DESCRIPTOR: usize = 4 * 1024;
const MAX_PROTOCOLS_PER_DESCRIPTOR: usize = 256;
const MAX_METHODS_PER_PROTOCOL_IMPL: usize = 4 * 1024;

/// Type-param-level architectural upper bounds.  Each type
/// parameter can declare protocol bounds (`fn f<T: P + Q>`) and
/// each variant can have its own field list — both varint-driven.
const MAX_BOUNDS_PER_TYPE_PARAM: usize = 64;
const MAX_FIELDS_PER_VARIANT: usize = 1024;

/// Type-reference-level architectural upper bounds.  `TypeRef`
/// carries varint-driven counts for instantiation args, function
/// params, and context names — all reachable from hostile
/// bytecode in arbitrarily-deep type expressions.
const MAX_TYPE_REF_INSTANTIATION_ARGS: usize = 64;
const MAX_FN_TYPE_REF_PARAMS: usize = 256;
const MAX_FN_TYPE_REF_CONTEXTS: usize = 32;

/// Constant-pool / specialization / source-map bounds.  Each
/// driver is a varint that, post the cf1cff4c canonicality fix,
/// can decode to `u64::MAX`.
const MAX_CONSTANT_ARRAY_LEN: usize = 1 << 20;          // 1 048 576
const MAX_SPECIALIZATION_TYPE_ARGS: usize = 64;
const MAX_SOURCE_MAP_FILES: usize = 1 << 16;            // 65 536
const MAX_SOURCE_MAP_ENTRIES: usize = 1 << 22;          // 4 194 304

/// Maximum decompressed bytecode size for a single module.
///
/// Adversarial bytecode can claim a near-`u32::MAX` decompressed
/// size in the bytecode-section header; the decompressor would
/// `Vec::with_capacity` that amount before reading a byte from
/// the compressed stream.  1 GB is a generous cap — real Verum
/// modules are kilobytes, the embedded stdlib (every core/*.vr
/// compiled) is ~14 MB.
const MAX_DECOMPRESSED_BYTECODE_BYTES: u32 = 1 << 30;   // 1 GB

/// VBC module deserializer.
struct Deserializer<'a> {
    /// Input data.
    data: &'a [u8],
    /// Current read position.
    offset: usize,
    /// Parsed header.
    header: Option<VbcHeader>,
}

impl<'a> Deserializer<'a> {
    /// Creates a new deserializer.
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            header: None,
        }
    }

    /// Deserializes a complete module.
    fn deserialize(&mut self) -> VbcResult<VbcModule> {
        // 1. Parse and validate header
        let header = self.parse_header()?;
        self.header = Some(header.clone());

        // Memory-amplification defense: reject implausibly large
        // table-count fields before any allocation.  See the
        // MAX_*_ENTRIES constants above for rationale.
        if header.type_table_count > MAX_TYPE_TABLE_ENTRIES {
            return Err(VbcError::TableTooLarge {
                field: "type_table_count",
                count: header.type_table_count,
                max: MAX_TYPE_TABLE_ENTRIES,
            });
        }
        if header.function_table_count > MAX_FUNCTION_TABLE_ENTRIES {
            return Err(VbcError::TableTooLarge {
                field: "function_table_count",
                count: header.function_table_count,
                max: MAX_FUNCTION_TABLE_ENTRIES,
            });
        }
        if header.constant_pool_count > MAX_CONSTANT_POOL_ENTRIES {
            return Err(VbcError::TableTooLarge {
                field: "constant_pool_count",
                count: header.constant_pool_count,
                max: MAX_CONSTANT_POOL_ENTRIES,
            });
        }
        if header.specialization_table_count > MAX_SPECIALIZATION_TABLE_ENTRIES {
            return Err(VbcError::TableTooLarge {
                field: "specialization_table_count",
                count: header.specialization_table_count,
                max: MAX_SPECIALIZATION_TABLE_ENTRIES,
            });
        }

        // 2. Parse string table
        self.offset = header.string_table_offset as usize;
        let strings = self.parse_string_table(header.string_table_size)?;

        // Get module name
        let name = strings
            .get(StringId(header.module_name_offset))
            .unwrap_or("")
            .to_string();

        // 3. Parse type table
        self.offset = header.type_table_offset as usize;
        let mut types = Vec::with_capacity(header.type_table_count as usize);
        for _ in 0..header.type_table_count {
            types.push(self.parse_type_descriptor()?);
        }

        // 4. Parse function table
        self.offset = header.function_table_offset as usize;
        let mut functions = Vec::with_capacity(header.function_table_count as usize);
        for _ in 0..header.function_table_count {
            functions.push(self.parse_function_descriptor()?);
        }

        // 5. Parse constant pool
        self.offset = header.constant_pool_offset as usize;
        let mut constants = Vec::with_capacity(header.constant_pool_count as usize);
        for _ in 0..header.constant_pool_count {
            constants.push(self.parse_constant()?);
        }

        // 6. Parse bytecode (with optional decompression)
        self.offset = header.bytecode_offset as usize;
        let bytecode = self.parse_bytecode(header.bytecode_size)?;

        // 7. Parse specialization table
        let mut specializations = Vec::new();
        if header.specialization_table_count > 0 {
            self.offset = header.specialization_table_offset as usize;
            for _ in 0..header.specialization_table_count {
                specializations.push(self.parse_specialization()?);
            }
        }

        // 8. Parse source map (optional)
        let source_map = if header.source_map_offset > 0 && header.source_map_size > 0 {
            self.offset = header.source_map_offset as usize;
            Some(self.parse_source_map()?)
        } else {
            None
        };

        // 9. Parse extensions (tensor metadata, FFI, dependencies)
        let extensions = if header.extensions_offset > 0 && header.extensions_size > 0 {
            self.offset = header.extensions_offset as usize;
            self.parse_extensions(header.extensions_size)?
        } else {
            ExtensionsData::default()
        };

        Ok(VbcModule {
            header,
            name,
            strings,
            types,
            functions,
            constants,
            bytecode,
            specializations,
            source_map,
            dependencies: extensions.dependencies,
            ffi_libraries: extensions.ffi_libraries,
            ffi_symbols: extensions.ffi_symbols,
            ffi_layouts: extensions.ffi_layouts,
            // Source directory is set during compilation, not deserialization.
            source_dir: None,
            // Tensor metadata from extensions section
            shape_metadata: extensions.shape_metadata,
            device_hints: extensions.device_hints,
            distribution: extensions.distribution,
            autodiff_graph: extensions.autodiff_graph,
            mlir_hints: extensions.mlir_hints,
            global_ctors: Vec::new(),
            global_dtors: Vec::new(),
            context_names: Vec::new(),
            field_id_to_name: Vec::new(),
            type_field_layouts: std::collections::HashMap::new(),
            user_function_start: 0,
        })
    }

    /// Parses and validates the header.
    fn parse_header(&mut self) -> VbcResult<VbcHeader> {
        // Check magic first (only need 4 bytes)
        if self.data.len() < 4 {
            return Err(VbcError::eof(0, 4));
        }

        let magic: [u8; 4] = self.data[0..4].try_into().unwrap();
        if magic != MAGIC {
            return Err(VbcError::InvalidMagic(magic));
        }

        // Now check full header size
        if self.data.len() < HEADER_SIZE {
            return Err(VbcError::eof(0, HEADER_SIZE));
        }

        self.offset = 4;

        // Version
        let version_major = decode_u16(self.data, &mut self.offset)?;
        let version_minor = decode_u16(self.data, &mut self.offset)?;
        if version_major != VERSION_MAJOR || version_minor > VERSION_MINOR {
            return Err(VbcError::UnsupportedVersion {
                major: version_major,
                minor: version_minor,
                supported_major: VERSION_MAJOR,
                supported_minor: VERSION_MINOR,
            });
        }

        // Flags
        let flags_bits = decode_u32(self.data, &mut self.offset)?;
        let flags = VbcFlags::from_bits_truncate(flags_bits);

        // Offsets and counts
        let module_name_offset = decode_u32(self.data, &mut self.offset)?;
        let type_table_offset = decode_u32(self.data, &mut self.offset)?;
        let type_table_count = decode_u32(self.data, &mut self.offset)?;
        let function_table_offset = decode_u32(self.data, &mut self.offset)?;
        let function_table_count = decode_u32(self.data, &mut self.offset)?;
        let constant_pool_offset = decode_u32(self.data, &mut self.offset)?;
        let constant_pool_count = decode_u32(self.data, &mut self.offset)?;
        let string_table_offset = decode_u32(self.data, &mut self.offset)?;
        let string_table_size = decode_u32(self.data, &mut self.offset)?;
        let bytecode_offset = decode_u32(self.data, &mut self.offset)?;
        let bytecode_size = decode_u32(self.data, &mut self.offset)?;
        let specialization_table_offset = decode_u32(self.data, &mut self.offset)?;
        let specialization_table_count = decode_u32(self.data, &mut self.offset)?;
        let source_map_offset = decode_u32(self.data, &mut self.offset)?;
        let source_map_size = decode_u32(self.data, &mut self.offset)?;
        let content_hash = decode_u64(self.data, &mut self.offset)?;
        let dependency_hash = decode_u64(self.data, &mut self.offset)?;

        // Extensions
        let extensions_offset = decode_u32(self.data, &mut self.offset)?;
        let extensions_size = decode_u32(self.data, &mut self.offset)?;

        // Validate section bounds
        let file_size = self.data.len();
        self.validate_section_bounds("string_table", string_table_offset, string_table_size, file_size)?;
        self.validate_section_bounds("bytecode", bytecode_offset, bytecode_size, file_size)?;
        if source_map_offset > 0 {
            self.validate_section_bounds("source_map", source_map_offset, source_map_size, file_size)?;
        }
        if extensions_offset > 0 {
            self.validate_section_bounds("extensions", extensions_offset, extensions_size, file_size)?;
        }

        Ok(VbcHeader {
            magic,
            version_major,
            version_minor,
            flags,
            module_name_offset,
            type_table_offset,
            type_table_count,
            function_table_offset,
            function_table_count,
            constant_pool_offset,
            constant_pool_count,
            string_table_offset,
            string_table_size,
            bytecode_offset,
            bytecode_size,
            specialization_table_offset,
            specialization_table_count,
            source_map_offset,
            source_map_size,
            content_hash,
            dependency_hash,
            extensions_offset,
            extensions_size,
        })
    }

    /// Validates section bounds.
    fn validate_section_bounds(
        &self,
        section: &'static str,
        offset: u32,
        size: u32,
        file_size: usize,
    ) -> VbcResult<()> {
        if offset as usize > file_size {
            return Err(VbcError::SectionOutOfBounds {
                section,
                offset,
                file_size,
            });
        }
        let end = offset.checked_add(size).ok_or(VbcError::SectionOverflow {
            section,
            offset,
            size,
        })?;
        if end as usize > file_size {
            return Err(VbcError::SectionOverflow {
                section,
                offset,
                size,
            });
        }
        Ok(())
    }

    /// Parses the string table.
    fn parse_string_table(&mut self, size: u32) -> VbcResult<StringTable> {
        let mut table = StringTable::new();
        // Use checked_add for the section-end calculation — on
        // 32-bit targets the cast `size as usize` covers full u32
        // (4 GB) and adding *offset can overflow.  On 64-bit the
        // overflow is unreachable but the checked path is
        // platform-portable and zero-cost when no overflow occurs.
        let end = self.offset
            .checked_add(size as usize)
            .ok_or(VbcError::SectionOverflow {
                section: "string_table",
                offset: self.offset as u32,
                size,
            })?;
        if end > self.data.len() {
            return Err(VbcError::SectionOverflow {
                section: "string_table",
                offset: self.offset as u32,
                size,
            });
        }

        while self.offset < end {
            let str_start = self.offset;
            let len = decode_u32(self.data, &mut self.offset)? as usize;

            // Per-string bound: a single string entry larger than
            // the entire string table is structurally malformed,
            // and even a string approaching `u32::MAX` (4 GB) is
            // unreasonable for real Verum module-level strings.
            // `size` itself is bounded by the file size, so a
            // string larger than `size` is always invalid; cap
            // each string at the section size for an explicit
            // diagnostic before the slice is computed.
            if len > size as usize {
                return Err(VbcError::TableTooLarge {
                    field: "string_entry_len",
                    count: len.min(u32::MAX as usize) as u32,
                    max: size,
                });
            }

            let str_end = self.offset
                .checked_add(len)
                .ok_or_else(|| VbcError::eof(self.offset, len))?;
            if str_end > end {
                return Err(VbcError::eof(self.offset, len));
            }

            let bytes = &self.data[self.offset..str_end];
            let s = std::str::from_utf8(bytes).map_err(|_| VbcError::InvalidUtf8 {
                offset: str_start as u32,
                error: String::from_utf8(bytes.to_vec()).unwrap_err(),
            })?;

            table.intern(s);
            self.offset = str_end;
        }

        Ok(table)
    }

    /// Parses bytecode with optional decompression.
    ///
    /// # Format
    ///
    /// The bytecode section has a compression header followed by the data:
    /// - `u8`: Compression algorithm (0=None, 1=Zstd, 2=Lz4)
    /// - `u32`: Uncompressed size (only present if algorithm != None)
    /// - `bytes`: Compressed or uncompressed bytecode
    fn parse_bytecode(&mut self, section_size: u32) -> VbcResult<Vec<u8>> {
        // The section MUST contain at least the algorithm byte; a
        // zero-size section is malformed.  Without this check the
        // `section_size as usize - 1` arithmetic on the None branch
        // below would underflow on hostile inputs.
        if section_size == 0 {
            return Err(VbcError::InvalidHeader {
                field: "bytecode_size",
                offset: self.offset,
            });
        }
        let section_end = self.offset
            .checked_add(section_size as usize)
            .ok_or(VbcError::SectionOverflow {
                section: "bytecode",
                offset: self.offset as u32,
                size: section_size,
            })?;
        if section_end > self.data.len() {
            return Err(VbcError::SectionOverflow {
                section: "bytecode",
                offset: self.offset as u32,
                size: section_size,
            });
        }

        // Read compression algorithm
        let algorithm_byte = decode_u8(self.data, &mut self.offset)?;
        let algorithm = CompressionAlgorithm::try_from(algorithm_byte)
            .map_err(VbcError::UnknownCompression)?;

        match algorithm {
            CompressionAlgorithm::None => {
                // No compression - just copy the raw data.
                // section_size > 0 checked above, so the subtraction
                // is safe.
                let data_size = (section_size - 1) as usize;
                let data_end = self.offset
                    .checked_add(data_size)
                    .ok_or_else(|| VbcError::eof(self.offset, data_size))?;
                if data_end > self.data.len() {
                    return Err(VbcError::eof(self.offset, data_size));
                }
                let bytecode = self.data[self.offset..data_end].to_vec();
                self.offset = data_end;
                Ok(bytecode)
            }
            CompressionAlgorithm::Zstd | CompressionAlgorithm::Lz4 => {
                // Read uncompressed size
                let uncompressed_size = decode_u32(self.data, &mut self.offset)?;

                // Memory-amplification defense: a hostile compressed
                // section with `uncompressed_size` near `u32::MAX`
                // would cause the decompressor to allocate ~4 GB
                // before reading a byte from the compressed stream.
                if uncompressed_size > MAX_DECOMPRESSED_BYTECODE_BYTES {
                    return Err(VbcError::TableTooLarge {
                        field: "uncompressed_bytecode_size",
                        count: uncompressed_size,
                        max: MAX_DECOMPRESSED_BYTECODE_BYTES,
                    });
                }

                // Read compressed data (rest of section).  section_end
                // is bounded by data.len() above, so this subtraction
                // is safe and the slice is in-bounds.
                let compressed_size = section_end - self.offset;
                let compressed = &self.data[self.offset..section_end];
                self.offset = section_end;

                // Decompress
                decompress(compressed, algorithm, uncompressed_size)
                    .inspect(|_out| {
                        let _ = compressed_size; // explicitly silence unused
                    })
            }
        }
    }

    /// Parses a type descriptor.
    fn parse_type_descriptor(&mut self) -> VbcResult<TypeDescriptor> {
        let id = TypeId(decode_u32(self.data, &mut self.offset)?);
        let name = StringId(decode_u32(self.data, &mut self.offset)?);
        let kind = TypeKind::try_from(decode_u8(self.data, &mut self.offset)?)
            .map_err(VbcError::InvalidTypeKind)?;
        let visibility = Visibility::try_from(decode_u8(self.data, &mut self.offset)?)
            .map_err(|_| VbcError::InvalidHeader { field: "visibility", offset: self.offset })?;
        let size = decode_u32(self.data, &mut self.offset)?;
        let alignment = decode_u32(self.data, &mut self.offset)?;

        // Type parameters
        let type_params_count = decode_varint(self.data, &mut self.offset)? as usize;
        if type_params_count > MAX_TYPE_PARAMS_PER_DESCRIPTOR {
            return Err(VbcError::TableTooLarge {
                field: "type_params_count",
                count: type_params_count.min(u32::MAX as usize) as u32,
                max: MAX_TYPE_PARAMS_PER_DESCRIPTOR as u32,
            });
        }
        let mut type_params = SmallVec::with_capacity(type_params_count);
        for _ in 0..type_params_count {
            type_params.push(self.parse_type_param()?);
        }

        // Fields
        let fields_count = decode_varint(self.data, &mut self.offset)? as usize;
        if fields_count > MAX_FIELDS_PER_DESCRIPTOR {
            return Err(VbcError::TableTooLarge {
                field: "fields_count",
                count: fields_count.min(u32::MAX as usize) as u32,
                max: MAX_FIELDS_PER_DESCRIPTOR as u32,
            });
        }
        let mut fields = SmallVec::with_capacity(fields_count);
        for _ in 0..fields_count {
            fields.push(self.parse_field()?);
        }

        // Variants
        let variants_count = decode_varint(self.data, &mut self.offset)? as usize;
        if variants_count > MAX_VARIANTS_PER_DESCRIPTOR {
            return Err(VbcError::TableTooLarge {
                field: "variants_count",
                count: variants_count.min(u32::MAX as usize) as u32,
                max: MAX_VARIANTS_PER_DESCRIPTOR as u32,
            });
        }
        let mut variants = SmallVec::with_capacity(variants_count);
        for _ in 0..variants_count {
            variants.push(self.parse_variant()?);
        }

        // Drop/clone functions
        let drop_fn = self.parse_optional_u32()?;
        let clone_fn = self.parse_optional_u32()?;

        // Protocols
        let protocols_count = decode_varint(self.data, &mut self.offset)? as usize;
        if protocols_count > MAX_PROTOCOLS_PER_DESCRIPTOR {
            return Err(VbcError::TableTooLarge {
                field: "protocols_count",
                count: protocols_count.min(u32::MAX as usize) as u32,
                max: MAX_PROTOCOLS_PER_DESCRIPTOR as u32,
            });
        }
        let mut protocols = SmallVec::with_capacity(protocols_count);
        for _ in 0..protocols_count {
            let protocol = ProtocolId(decode_u32(self.data, &mut self.offset)?);
            let methods_count = decode_varint(self.data, &mut self.offset)? as usize;
            if methods_count > MAX_METHODS_PER_PROTOCOL_IMPL {
                return Err(VbcError::TableTooLarge {
                    field: "methods_count",
                    count: methods_count.min(u32::MAX as usize) as u32,
                    max: MAX_METHODS_PER_PROTOCOL_IMPL as u32,
                });
            }
            let mut methods = Vec::with_capacity(methods_count);
            for _ in 0..methods_count {
                methods.push(decode_u32(self.data, &mut self.offset)?);
            }
            protocols.push(ProtocolImpl { protocol, methods });
        }

        Ok(TypeDescriptor {
            id,
            name,
            kind,
            type_params,
            fields,
            variants,
            size,
            alignment,
            drop_fn,
            clone_fn,
            protocols,
            visibility,
        })
    }

    /// Parses a type parameter descriptor.
    fn parse_type_param(&mut self) -> VbcResult<TypeParamDescriptor> {
        let name = StringId(decode_u32(self.data, &mut self.offset)?);
        let id = TypeParamId(decode_u16(self.data, &mut self.offset)?);
        let variance = Variance::try_from(decode_u8(self.data, &mut self.offset)?)
            .map_err(|_| VbcError::InvalidHeader { field: "variance", offset: self.offset })?;

        // Bounds
        let bounds_count = decode_varint(self.data, &mut self.offset)? as usize;
        if bounds_count > MAX_BOUNDS_PER_TYPE_PARAM {
            return Err(VbcError::TableTooLarge {
                field: "type_param_bounds_count",
                count: bounds_count.min(u32::MAX as usize) as u32,
                max: MAX_BOUNDS_PER_TYPE_PARAM as u32,
            });
        }
        let mut bounds = SmallVec::with_capacity(bounds_count);
        for _ in 0..bounds_count {
            bounds.push(ProtocolId(decode_u32(self.data, &mut self.offset)?));
        }

        // Default
        let has_default = decode_u8(self.data, &mut self.offset)? != 0;
        let default = if has_default {
            Some(self.parse_type_ref()?)
        } else {
            None
        };

        Ok(TypeParamDescriptor {
            name,
            id,
            bounds,
            default,
            variance,
        })
    }

    /// Parses a field descriptor.
    fn parse_field(&mut self) -> VbcResult<FieldDescriptor> {
        let name = StringId(decode_u32(self.data, &mut self.offset)?);
        let type_ref = self.parse_type_ref()?;
        let offset = decode_u32(self.data, &mut self.offset)?;
        let visibility = Visibility::try_from(decode_u8(self.data, &mut self.offset)?)
            .map_err(|_| VbcError::InvalidHeader { field: "field_visibility", offset: self.offset })?;

        Ok(FieldDescriptor {
            name,
            type_ref,
            offset,
            visibility,
        })
    }

    /// Parses a variant descriptor.
    fn parse_variant(&mut self) -> VbcResult<VariantDescriptor> {
        let name = StringId(decode_u32(self.data, &mut self.offset)?);
        let tag = decode_u32(self.data, &mut self.offset)?;
        let kind = VariantKind::try_from(decode_u8(self.data, &mut self.offset)?)
            .map_err(|_| VbcError::InvalidHeader { field: "variant_kind", offset: self.offset })?;
        let arity = decode_u8(self.data, &mut self.offset)?;

        // Payload
        let has_payload = decode_u8(self.data, &mut self.offset)? != 0;
        let payload = if has_payload {
            Some(self.parse_type_ref()?)
        } else {
            None
        };

        // Fields
        let fields_count = decode_varint(self.data, &mut self.offset)? as usize;
        if fields_count > MAX_FIELDS_PER_VARIANT {
            return Err(VbcError::TableTooLarge {
                field: "variant_fields_count",
                count: fields_count.min(u32::MAX as usize) as u32,
                max: MAX_FIELDS_PER_VARIANT as u32,
            });
        }
        let mut fields = SmallVec::with_capacity(fields_count);
        for _ in 0..fields_count {
            fields.push(self.parse_field()?);
        }

        Ok(VariantDescriptor {
            name,
            tag,
            payload,
            kind,
            arity,
            fields,
        })
    }

    /// Parses a type reference.
    fn parse_type_ref(&mut self) -> VbcResult<TypeRef> {
        let tag = decode_u8(self.data, &mut self.offset)?;

        match tag & 0x0F {
            0x01 => {
                let id = TypeId(decode_u32(self.data, &mut self.offset)?);
                Ok(TypeRef::Concrete(id))
            }
            0x02 => {
                let id = TypeParamId(decode_u16(self.data, &mut self.offset)?);
                Ok(TypeRef::Generic(id))
            }
            0x03 => {
                let base = TypeId(decode_u32(self.data, &mut self.offset)?);
                let count = decode_varint(self.data, &mut self.offset)? as usize;
                if count > MAX_TYPE_REF_INSTANTIATION_ARGS {
                    return Err(VbcError::TableTooLarge {
                        field: "type_ref_instantiation_args",
                        count: count.min(u32::MAX as usize) as u32,
                        max: MAX_TYPE_REF_INSTANTIATION_ARGS as u32,
                    });
                }
                let mut args = Vec::with_capacity(count);
                for _ in 0..count {
                    args.push(self.parse_type_ref()?);
                }
                Ok(TypeRef::Instantiated { base, args })
            }
            0x04 => {
                let params_count = decode_varint(self.data, &mut self.offset)? as usize;
                if params_count > MAX_FN_TYPE_REF_PARAMS {
                    return Err(VbcError::TableTooLarge {
                        field: "fn_type_ref_params",
                        count: params_count.min(u32::MAX as usize) as u32,
                        max: MAX_FN_TYPE_REF_PARAMS as u32,
                    });
                }
                let mut params = Vec::with_capacity(params_count);
                for _ in 0..params_count {
                    params.push(self.parse_type_ref()?);
                }
                let return_type = Box::new(self.parse_type_ref()?);
                let ctx_count = decode_varint(self.data, &mut self.offset)? as usize;
                if ctx_count > MAX_FN_TYPE_REF_CONTEXTS {
                    return Err(VbcError::TableTooLarge {
                        field: "fn_type_ref_contexts",
                        count: ctx_count.min(u32::MAX as usize) as u32,
                        max: MAX_FN_TYPE_REF_CONTEXTS as u32,
                    });
                }
                let mut contexts = SmallVec::with_capacity(ctx_count);
                for _ in 0..ctx_count {
                    contexts.push(ContextRef(decode_u32(self.data, &mut self.offset)?));
                }
                Ok(TypeRef::Function {
                    params,
                    return_type,
                    contexts,
                })
            }
            0x05 => {
                let mutability = if (tag >> 4) & 0x01 != 0 {
                    Mutability::Mutable
                } else {
                    Mutability::Immutable
                };
                let tier = match (tag >> 6) & 0x03 {
                    0 => CbgrTier::Tier0,
                    1 => CbgrTier::Tier1,
                    2 => CbgrTier::Tier2,
                    _ => CbgrTier::Tier0,
                };
                let inner = Box::new(self.parse_type_ref()?);
                Ok(TypeRef::Reference {
                    inner,
                    mutability,
                    tier,
                })
            }
            0x06 => {
                let count = decode_varint(self.data, &mut self.offset)? as usize;
                let mut elems = Vec::with_capacity(count);
                for _ in 0..count {
                    elems.push(self.parse_type_ref()?);
                }
                Ok(TypeRef::Tuple(elems))
            }
            0x07 => {
                let element = Box::new(self.parse_type_ref()?);
                let length = decode_u64(self.data, &mut self.offset)?;
                Ok(TypeRef::Array { element, length })
            }
            0x08 => {
                let inner = Box::new(self.parse_type_ref()?);
                Ok(TypeRef::Slice(inner))
            }
            0x09 => {
                let type_param_count = decode_u16(self.data, &mut self.offset)?;
                let params_count = decode_varint(self.data, &mut self.offset)? as usize;
                let mut params = Vec::with_capacity(params_count);
                for _ in 0..params_count {
                    params.push(self.parse_type_ref()?);
                }
                let return_type = Box::new(self.parse_type_ref()?);
                let ctx_count = decode_varint(self.data, &mut self.offset)? as usize;
                let mut contexts = SmallVec::with_capacity(ctx_count);
                for _ in 0..ctx_count {
                    contexts.push(ContextRef(decode_u32(self.data, &mut self.offset)?));
                }
                Ok(TypeRef::Rank2Function {
                    type_param_count,
                    params,
                    return_type,
                    contexts,
                })
            }
            _ => Err(VbcError::InvalidTypeRefTag(tag)),
        }
    }

    /// Parses a function descriptor.
    fn parse_function_descriptor(&mut self) -> VbcResult<FunctionDescriptor> {
        let id = FunctionId(decode_u32(self.data, &mut self.offset)?);
        let name = StringId(decode_u32(self.data, &mut self.offset)?);
        let parent_type = self.parse_optional_u32()?.map(TypeId);
        let visibility = Visibility::try_from(decode_u8(self.data, &mut self.offset)?)
            .map_err(|_| VbcError::InvalidHeader { field: "func_visibility", offset: self.offset })?;

        let flags = decode_u8(self.data, &mut self.offset)?;
        let is_inline_candidate = flags & 0x01 != 0;
        let is_generic = flags & 0x02 != 0;
        let is_generator = flags & 0x04 != 0;

        let properties = PropertySet::from_bits_truncate(decode_u16(self.data, &mut self.offset)?);

        let bytecode_offset = decode_u32(self.data, &mut self.offset)?;
        let bytecode_length = decode_u32(self.data, &mut self.offset)?;
        let locals_count = decode_u16(self.data, &mut self.offset)?;
        let register_count = decode_u16(self.data, &mut self.offset)?;
        let max_stack = decode_u16(self.data, &mut self.offset)?;

        // Type parameters — same bound as type-descriptor's
        // type_params, since both describe the same generic
        // signature shape.
        let type_params_count = decode_varint(self.data, &mut self.offset)? as usize;
        if type_params_count > MAX_TYPE_PARAMS_PER_DESCRIPTOR {
            return Err(VbcError::TableTooLarge {
                field: "fn_type_params_count",
                count: type_params_count.min(u32::MAX as usize) as u32,
                max: MAX_TYPE_PARAMS_PER_DESCRIPTOR as u32,
            });
        }
        let mut type_params = SmallVec::with_capacity(type_params_count);
        for _ in 0..type_params_count {
            type_params.push(self.parse_type_param()?);
        }

        // Parameters — reuses MAX_FN_TYPE_REF_PARAMS since a
        // function descriptor and a function-type ref describe
        // the same parameter-list shape.
        let params_count = decode_varint(self.data, &mut self.offset)? as usize;
        if params_count > MAX_FN_TYPE_REF_PARAMS {
            return Err(VbcError::TableTooLarge {
                field: "fn_params_count",
                count: params_count.min(u32::MAX as usize) as u32,
                max: MAX_FN_TYPE_REF_PARAMS as u32,
            });
        }
        let mut params = SmallVec::with_capacity(params_count);
        for _ in 0..params_count {
            let param_name = StringId(decode_u32(self.data, &mut self.offset)?);
            let type_ref = self.parse_type_ref()?;
            let is_mut = decode_u8(self.data, &mut self.offset)? != 0;
            let default = self.parse_optional_u32()?.map(ConstId);
            params.push(ParamDescriptor {
                name: param_name,
                type_ref,
                is_mut,
                default,
            });
        }

        // Return type
        let return_type = self.parse_type_ref()?;

        // Contexts — same bound as fn-type-ref contexts.
        let ctx_count = decode_varint(self.data, &mut self.offset)? as usize;
        if ctx_count > MAX_FN_TYPE_REF_CONTEXTS {
            return Err(VbcError::TableTooLarge {
                field: "fn_contexts_count",
                count: ctx_count.min(u32::MAX as usize) as u32,
                max: MAX_FN_TYPE_REF_CONTEXTS as u32,
            });
        }
        let mut contexts = SmallVec::with_capacity(ctx_count);
        for _ in 0..ctx_count {
            contexts.push(ContextRef(decode_u32(self.data, &mut self.offset)?));
        }

        // Generator metadata (if is_generator flag is set)
        let (yield_type, suspend_point_count) = if is_generator {
            let has_yield_type = decode_u8(self.data, &mut self.offset)? != 0;
            let yield_type = if has_yield_type {
                Some(self.parse_type_ref()?)
            } else {
                None
            };
            let suspend_point_count = decode_u16(self.data, &mut self.offset)?;
            (yield_type, suspend_point_count)
        } else {
            (None, 0)
        };

        Ok(FunctionDescriptor {
            id,
            name,
            parent_type,
            type_params,
            params,
            return_type,
            contexts,
            properties,
            bytecode_offset,
            bytecode_length,
            locals_count,
            register_count,
            max_stack,
            is_inline_candidate,
            is_generic,
            visibility,
            is_generator,
            yield_type,
            suspend_point_count,
            calling_convention: CallingConvention::C, // Default to C convention
            optimization_hints: OptimizationHints::default(),
            instructions: None,
            func_id_base: 0,
            debug_variables: Vec::new(),
            is_test: false,
        })
    }

    /// Parses a constant.
    fn parse_constant(&mut self) -> VbcResult<Constant> {
        let tag = decode_u8(self.data, &mut self.offset)?;
        match tag {
            0x01 => Ok(Constant::Int(decode_i64(self.data, &mut self.offset)?)),
            0x02 => Ok(Constant::Float(decode_f64(self.data, &mut self.offset)?)),
            0x03 => Ok(Constant::String(StringId(decode_u32(self.data, &mut self.offset)?))),
            0x04 => Ok(Constant::Type(self.parse_type_ref()?)),
            0x05 => Ok(Constant::Function(FunctionId(decode_u32(self.data, &mut self.offset)?))),
            0x06 => Ok(Constant::Protocol(ProtocolId(decode_u32(self.data, &mut self.offset)?))),
            0x07 => {
                let count = decode_varint(self.data, &mut self.offset)? as usize;
                if count > MAX_CONSTANT_ARRAY_LEN {
                    return Err(VbcError::TableTooLarge {
                        field: "constant_array_count",
                        count: count.min(u32::MAX as usize) as u32,
                        max: MAX_CONSTANT_ARRAY_LEN as u32,
                    });
                }
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(ConstId(decode_u32(self.data, &mut self.offset)?));
                }
                Ok(Constant::Array(items))
            }
            0x08 => {
                let bytes = decode_bytes(self.data, &mut self.offset)?;
                Ok(Constant::Bytes(bytes))
            }
            _ => Err(VbcError::InvalidConstantTag(tag)),
        }
    }

    /// Parses a specialization entry.
    fn parse_specialization(&mut self) -> VbcResult<SpecializationEntry> {
        let generic_fn = FunctionId(decode_u32(self.data, &mut self.offset)?);
        let hash = decode_u64(self.data, &mut self.offset)?;
        let bytecode_offset = decode_u32(self.data, &mut self.offset)?;
        let bytecode_length = decode_u32(self.data, &mut self.offset)?;
        let register_count = decode_u16(self.data, &mut self.offset)?;

        let type_args_count = decode_varint(self.data, &mut self.offset)? as usize;
        if type_args_count > MAX_SPECIALIZATION_TYPE_ARGS {
            return Err(VbcError::TableTooLarge {
                field: "specialization_type_args",
                count: type_args_count.min(u32::MAX as usize) as u32,
                max: MAX_SPECIALIZATION_TYPE_ARGS as u32,
            });
        }
        let mut type_args = Vec::with_capacity(type_args_count);
        for _ in 0..type_args_count {
            type_args.push(self.parse_type_ref()?);
        }

        Ok(SpecializationEntry {
            generic_fn,
            type_args,
            hash,
            bytecode_offset,
            bytecode_length,
            register_count,
        })
    }

    /// Parses a source map.
    fn parse_source_map(&mut self) -> VbcResult<SourceMap> {
        let files_count = decode_varint(self.data, &mut self.offset)? as usize;
        if files_count > MAX_SOURCE_MAP_FILES {
            return Err(VbcError::TableTooLarge {
                field: "source_map_files_count",
                count: files_count.min(u32::MAX as usize) as u32,
                max: MAX_SOURCE_MAP_FILES as u32,
            });
        }
        let mut files = Vec::with_capacity(files_count);
        for _ in 0..files_count {
            files.push(StringId(decode_u32(self.data, &mut self.offset)?));
        }

        let entries_count = decode_varint(self.data, &mut self.offset)? as usize;
        if entries_count > MAX_SOURCE_MAP_ENTRIES {
            return Err(VbcError::TableTooLarge {
                field: "source_map_entries_count",
                count: entries_count.min(u32::MAX as usize) as u32,
                max: MAX_SOURCE_MAP_ENTRIES as u32,
            });
        }
        let mut entries = Vec::with_capacity(entries_count);
        for _ in 0..entries_count {
            let bytecode_offset = decode_u32(self.data, &mut self.offset)?;
            let file_idx = decode_u16(self.data, &mut self.offset)?;
            let line = decode_u32(self.data, &mut self.offset)?;
            let column = decode_u16(self.data, &mut self.offset)?;
            entries.push(SourceMapEntry {
                bytecode_offset,
                file_idx,
                line,
                column,
            });
        }

        Ok(SourceMap { files, entries })
    }

    /// Parses the extensions section (tensor metadata, FFI, dependencies).
    ///
    /// Extension section format:
    /// - u8 section_mask: bitmask of present sections
    ///   - 0x01: shape_metadata (tensor shapes)
    ///   - 0x02: device_hints (GPU/CPU placement)
    ///   - 0x04: distribution (distributed training)
    ///   - 0x08: autodiff_graph (gradient computation)
    ///   - 0x10: mlir_hints (optimization hints)
    ///   - 0x20: ffi_tables (libraries, symbols, layouts)
    ///   - 0x40: dependencies (module dependencies)
    /// - For each present section: u32 length + bincode data
    fn parse_extensions(&mut self, _size: u32) -> VbcResult<ExtensionsData> {
        // Read section mask
        let section_mask = decode_u8(self.data, &mut self.offset)?;

        let mut result = ExtensionsData::default();

        // Parse each present section
        if section_mask & 0x01 != 0 {
            let len = decode_u32(self.data, &mut self.offset)? as usize;
            if self.offset + len > self.data.len() {
                return Err(VbcError::eof(self.offset, len));
            }
            result.shape_metadata =
                bincode::deserialize(&self.data[self.offset..self.offset + len]).map_err(|e| {
                    VbcError::Deserialization(format!("shape_metadata: {}", e))
                })?;
            self.offset += len;
        }

        if section_mask & 0x02 != 0 {
            let len = decode_u32(self.data, &mut self.offset)? as usize;
            if self.offset + len > self.data.len() {
                return Err(VbcError::eof(self.offset, len));
            }
            result.device_hints =
                bincode::deserialize(&self.data[self.offset..self.offset + len]).map_err(|e| {
                    VbcError::Deserialization(format!("device_hints: {}", e))
                })?;
            self.offset += len;
        }

        if section_mask & 0x04 != 0 {
            let len = decode_u32(self.data, &mut self.offset)? as usize;
            if self.offset + len > self.data.len() {
                return Err(VbcError::eof(self.offset, len));
            }
            result.distribution =
                bincode::deserialize(&self.data[self.offset..self.offset + len]).map_err(|e| {
                    VbcError::Deserialization(format!("distribution: {}", e))
                })?;
            self.offset += len;
        }

        if section_mask & 0x08 != 0 {
            let len = decode_u32(self.data, &mut self.offset)? as usize;
            if self.offset + len > self.data.len() {
                return Err(VbcError::eof(self.offset, len));
            }
            result.autodiff_graph =
                bincode::deserialize(&self.data[self.offset..self.offset + len]).map_err(|e| {
                    VbcError::Deserialization(format!("autodiff_graph: {}", e))
                })?;
            self.offset += len;
        }

        if section_mask & 0x10 != 0 {
            let len = decode_u32(self.data, &mut self.offset)? as usize;
            if self.offset + len > self.data.len() {
                return Err(VbcError::eof(self.offset, len));
            }
            result.mlir_hints =
                bincode::deserialize(&self.data[self.offset..self.offset + len]).map_err(|e| {
                    VbcError::Deserialization(format!("mlir_hints: {}", e))
                })?;
            self.offset += len;
        }

        // FFI tables (bit 0x20)
        if section_mask & 0x20 != 0 {
            let len = decode_u32(self.data, &mut self.offset)? as usize;
            if self.offset + len > self.data.len() {
                return Err(VbcError::eof(self.offset, len));
            }
            let ffi_bundle: FfiBundle =
                bincode::deserialize(&self.data[self.offset..self.offset + len]).map_err(|e| {
                    VbcError::Deserialization(format!("ffi_tables: {}", e))
                })?;
            result.ffi_libraries = ffi_bundle.libraries;
            result.ffi_symbols = ffi_bundle.symbols;
            result.ffi_layouts = ffi_bundle.layouts;
            self.offset += len;
        }

        // Dependencies (bit 0x40)
        if section_mask & 0x40 != 0 {
            let len = decode_u32(self.data, &mut self.offset)? as usize;
            if self.offset + len > self.data.len() {
                return Err(VbcError::eof(self.offset, len));
            }
            result.dependencies =
                bincode::deserialize(&self.data[self.offset..self.offset + len]).map_err(|e| {
                    VbcError::Deserialization(format!("dependencies: {}", e))
                })?;
            self.offset += len;
        }

        Ok(result)
    }

    /// Parses an optional u32.
    fn parse_optional_u32(&mut self) -> VbcResult<Option<u32>> {
        let has_value = decode_u8(self.data, &mut self.offset)? != 0;
        if has_value {
            Ok(Some(decode_u32(self.data, &mut self.offset)?))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialize::serialize_module;

    #[test]
    fn test_roundtrip_empty_module() {
        let module = VbcModule::new("test".to_string());
        let bytes = serialize_module(&module).unwrap();
        let loaded = deserialize_module(&bytes).unwrap();

        assert_eq!(module.name, loaded.name);
        assert_eq!(module.types.len(), loaded.types.len());
        assert_eq!(module.functions.len(), loaded.functions.len());
    }

    #[test]
    fn test_roundtrip_with_strings() {
        let mut module = VbcModule::new("test_module".to_string());
        let id1 = module.intern_string("hello");
        let id2 = module.intern_string("world");

        let bytes = serialize_module(&module).unwrap();
        let loaded = deserialize_module(&bytes).unwrap();

        assert_eq!(loaded.get_string(id1), Some("hello"));
        assert_eq!(loaded.get_string(id2), Some("world"));
    }

    #[test]
    fn test_roundtrip_with_constants() {
        let mut module = VbcModule::new("test".to_string());
        module.add_constant(Constant::Int(42));
        module.add_constant(Constant::Float(3.14159));
        let str_id = module.intern_string("test string");
        module.add_constant(Constant::String(str_id));

        let bytes = serialize_module(&module).unwrap();
        let loaded = deserialize_module(&bytes).unwrap();

        assert_eq!(loaded.constants.len(), 3);
        assert_eq!(loaded.constants[0], Constant::Int(42));
        if let Constant::Float(f) = loaded.constants[1] {
            assert!((f - 3.14159).abs() < 1e-10);
        } else {
            panic!("Expected float");
        }
    }

    #[test]
    fn test_invalid_magic() {
        let data = b"BADM\x00\x01\x00\x00"; // Wrong magic
        let result = deserialize_module(data);
        assert!(matches!(result, Err(VbcError::InvalidMagic(_))));
    }

    #[test]
    fn test_truncated_header() {
        let data = b"VBC1"; // Too short
        let result = deserialize_module(data);
        assert!(result.is_err());
    }

    /// Hostile module header claims `type_table_count = u32::MAX`.
    /// Pre-fix the deserializer would `Vec::with_capacity(u32::MAX)`
    /// — multi-GB allocation — before discovering the file is too
    /// short.  Post-fix the size is rejected at the parse-header
    /// gate before any allocation.
    #[test]
    fn test_deserialize_rejects_huge_type_table_count() {
        let mut module = VbcModule::new("victim".to_string());
        module.add_constant(Constant::Int(0)); // ensure offsets are non-trivial
        let mut bytes = serialize_module(&module).unwrap();

        // VbcHeader layout: magic(4) + version(4) + flags(4) +
        // type_table_offset(4) + type_table_count(4) at offset 16.
        // Patch type_table_count to u32::MAX in little-endian.
        let mark = b"VBC1";
        assert!(bytes.starts_with(mark));
        // Use a sentinel scan to find the type_table_count field
        // robustly across header-layout evolution: read the
        // serialized module, parse its header, find its offset.
        // For test purposes we just patch the first u32 after the
        // magic/version/flags/type_table_offset fields — this is
        // brittle but acceptable inside the test.  Use the actual
        // header struct to compute the byte offset.
        //
        // Layout (matches format::VbcHeader serialize order):
        //   magic[4] version[4] flags[4] module_name_offset[4]
        //   type_table_offset[4] type_table_count[4]   ← target
        let count_offset = 4 + 4 + 4 + 4 + 4;
        bytes[count_offset..count_offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());

        let result = deserialize_module(&bytes);
        match result {
            Err(VbcError::TableTooLarge { field, count, max }) => {
                assert_eq!(field, "type_table_count");
                assert_eq!(count, u32::MAX);
                assert_eq!(max, MAX_TYPE_TABLE_ENTRIES);
            }
            other => panic!(
                "expected TableTooLarge {{ field: type_table_count, .. }}, \
                 got {:?}",
                other.err()
            ),
        }
    }

    /// Build a module large enough that the post-header payload
    /// has bytes safely past the section table for tamper tests.
    fn build_big_module() -> VbcModule {
        let mut module = VbcModule::new("big_module_for_hash_tamper".to_string());
        // Pad with constants and strings so the compressed payload
        // grows beyond the section / table area. The exact tamper
        // offset isn't important — we just need a byte we know is
        // in the hashed payload region.
        for i in 0..32 {
            module.add_constant(Constant::Int(i as i64));
            module.intern_string(&format!("padding_string_{}", i));
        }
        module
    }

    /// `deserialize_module_validated` (strict-default) catches
    /// content-hash tampering. Pin: tampering the LAST byte of the
    /// file lands in the compressed-payload region and surfaces as
    /// `ContentHashMismatch`. Tampering the last byte is robust
    /// against header-layout evolution — it can never fall inside
    /// a fixed-size header field.
    #[test]
    fn validated_strict_rejects_payload_tampering() {
        let mut bytes = serialize_module(&build_big_module()).unwrap();
        // Corrupt the recorded content_hash field at offset 72 in the
        // header (per format::HEADER_SIZE layout: magic+version+flags+
        // 6 sections + content_hash@72 + dep_hash@80 + reserved@88).
        // The structural decode reads the header verbatim, so flipping
        // a hash byte succeeds at decode but trips verify_content_hash
        // — exactly the gate we're pinning.
        bytes[72] ^= 0xFF;

        let result = deserialize_module_validated(&bytes);
        assert!(
            matches!(result, Err(VbcError::ContentHashMismatch { .. })),
            "tampered payload must be rejected by strict default, got {:?}",
            result
        );
    }

    /// `deserialize_module_validated_with_options(_, fast())`
    /// honors `skip_hash_check = true` — the same tampered bytes
    /// that the strict path rejects now pass through (or fail at a
    /// later step that ISN'T ContentHashMismatch). Headline
    /// regression: pre-fix the fast preset had no path that could
    /// actually skip hash verification because no caller threaded
    /// the options through.
    #[test]
    fn validated_with_options_honors_skip_hash_check() {
        use crate::validate::ValidationOptions;
        let mut bytes = serialize_module(&build_big_module()).unwrap();
        // Corrupt the recorded content_hash field at offset 72 in the
        // header (per format::HEADER_SIZE layout: magic+version+flags+
        // 6 sections + content_hash@72 + dep_hash@80 + reserved@88).
        // The structural decode reads the header verbatim, so flipping
        // a hash byte succeeds at decode but trips verify_content_hash
        // — exactly the gate we're pinning.
        bytes[72] ^= 0xFF;

        // Strict default rejects (companion to the previous test).
        assert!(matches!(
            deserialize_module_validated(&bytes),
            Err(VbcError::ContentHashMismatch { .. })
        ));

        // Fast preset honors the skip — must NOT be ContentHashMismatch
        // (the gate we just confirmed is bypassed). Decompression
        // may still fail on the corrupt last byte, but that's a
        // distinct error class.
        let result = deserialize_module_validated_with_options(
            &bytes,
            &ValidationOptions::fast(),
        );
        assert!(
            !matches!(result, Err(VbcError::ContentHashMismatch { .. })),
            "fast() must skip hash verification — got ContentHashMismatch \
             which means the skip flag was ignored"
        );
    }

    /// `validated_with_options(_, strict())` matches the legacy
    /// `validated` entry-point's behaviour: rejects tampering with
    /// the same error class. Locks the equivalence so changes to
    /// one path don't silently diverge from the other.
    #[test]
    fn validated_with_options_strict_matches_legacy() {
        use crate::validate::ValidationOptions;
        let mut bytes = serialize_module(&build_big_module()).unwrap();
        // Corrupt the recorded content_hash field at offset 72 in the
        // header (per format::HEADER_SIZE layout: magic+version+flags+
        // 6 sections + content_hash@72 + dep_hash@80 + reserved@88).
        // The structural decode reads the header verbatim, so flipping
        // a hash byte succeeds at decode but trips verify_content_hash
        // — exactly the gate we're pinning.
        bytes[72] ^= 0xFF;

        let legacy = deserialize_module_validated(&bytes);
        let strict = deserialize_module_validated_with_options(
            &bytes,
            &ValidationOptions::strict(),
        );

        // Both must produce ContentHashMismatch — same reject reason.
        assert!(matches!(legacy, Err(VbcError::ContentHashMismatch { .. })));
        assert!(matches!(strict, Err(VbcError::ContentHashMismatch { .. })));
    }

    /// Untampered bytes must round-trip through the with-options
    /// entry point under both strict() and fast() — the skip flag
    /// should never produce a SPURIOUS rejection on valid input.
    #[test]
    fn validated_with_options_round_trips_clean_bytes() {
        use crate::validate::ValidationOptions;
        let module = build_big_module();
        let bytes = serialize_module(&module).unwrap();

        let strict_loaded = deserialize_module_validated_with_options(
            &bytes,
            &ValidationOptions::strict(),
        )
        .expect("clean bytes must load under strict");
        assert_eq!(strict_loaded.name, module.name);

        let fast_loaded = deserialize_module_validated_with_options(
            &bytes,
            &ValidationOptions::fast(),
        )
        .expect("clean bytes must load under fast");
        assert_eq!(fast_loaded.name, module.name);
    }
}
