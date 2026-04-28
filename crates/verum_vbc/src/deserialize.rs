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
    let module = deserialize_module(data)?;
    verify_content_hash(data, module.header.content_hash)?;
    verify_dependency_hash(&module)?;
    crate::validate::validate_module(&module)?;
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
        let end = self.offset + size as usize;

        while self.offset < end {
            let str_start = self.offset;
            let len = decode_u32(self.data, &mut self.offset)? as usize;

            if self.offset + len > end {
                return Err(VbcError::eof(self.offset, len));
            }

            let bytes = &self.data[self.offset..self.offset + len];
            let s = std::str::from_utf8(bytes).map_err(|_| VbcError::InvalidUtf8 {
                offset: str_start as u32,
                error: String::from_utf8(bytes.to_vec()).unwrap_err(),
            })?;

            table.intern(s);
            self.offset += len;
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
        let section_end = self.offset + section_size as usize;
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
                // No compression - just copy the raw data
                let data_size = section_size as usize - 1; // minus algorithm byte
                if self.offset + data_size > self.data.len() {
                    return Err(VbcError::eof(self.offset, data_size));
                }
                let bytecode = self.data[self.offset..self.offset + data_size].to_vec();
                self.offset += data_size;
                Ok(bytecode)
            }
            CompressionAlgorithm::Zstd | CompressionAlgorithm::Lz4 => {
                // Read uncompressed size
                let uncompressed_size = decode_u32(self.data, &mut self.offset)?;

                // Read compressed data (rest of section)
                let compressed_size = section_end - self.offset;
                if self.offset + compressed_size > self.data.len() {
                    return Err(VbcError::eof(self.offset, compressed_size));
                }
                let compressed = &self.data[self.offset..self.offset + compressed_size];
                self.offset += compressed_size;

                // Decompress
                decompress(compressed, algorithm, uncompressed_size)
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
        let mut type_params = SmallVec::with_capacity(type_params_count);
        for _ in 0..type_params_count {
            type_params.push(self.parse_type_param()?);
        }

        // Fields
        let fields_count = decode_varint(self.data, &mut self.offset)? as usize;
        let mut fields = SmallVec::with_capacity(fields_count);
        for _ in 0..fields_count {
            fields.push(self.parse_field()?);
        }

        // Variants
        let variants_count = decode_varint(self.data, &mut self.offset)? as usize;
        let mut variants = SmallVec::with_capacity(variants_count);
        for _ in 0..variants_count {
            variants.push(self.parse_variant()?);
        }

        // Drop/clone functions
        let drop_fn = self.parse_optional_u32()?;
        let clone_fn = self.parse_optional_u32()?;

        // Protocols
        let protocols_count = decode_varint(self.data, &mut self.offset)? as usize;
        let mut protocols = SmallVec::with_capacity(protocols_count);
        for _ in 0..protocols_count {
            let protocol = ProtocolId(decode_u32(self.data, &mut self.offset)?);
            let methods_count = decode_varint(self.data, &mut self.offset)? as usize;
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
                let mut args = Vec::with_capacity(count);
                for _ in 0..count {
                    args.push(self.parse_type_ref()?);
                }
                Ok(TypeRef::Instantiated { base, args })
            }
            0x04 => {
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

        // Type parameters
        let type_params_count = decode_varint(self.data, &mut self.offset)? as usize;
        let mut type_params = SmallVec::with_capacity(type_params_count);
        for _ in 0..type_params_count {
            type_params.push(self.parse_type_param()?);
        }

        // Parameters
        let params_count = decode_varint(self.data, &mut self.offset)? as usize;
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

        // Contexts
        let ctx_count = decode_varint(self.data, &mut self.offset)? as usize;
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
        let mut files = Vec::with_capacity(files_count);
        for _ in 0..files_count {
            files.push(StringId(decode_u32(self.data, &mut self.offset)?));
        }

        let entries_count = decode_varint(self.data, &mut self.offset)? as usize;
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
}
