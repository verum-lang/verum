//! VBC module serialization.
//!
//! This module provides serialization of VBC modules to binary format.
//! The output is a self-contained `.vbc` file that can be loaded for
//! interpretation, JIT, or AOT compilation.
//!
//! ## Compression Support
//!
//! VBC files can be compressed using zstd or lz4 to reduce storage and improve
//! load times (CPU-bound decompression is often faster than disk I/O for
//! compressed data).
//!
//! ```rust,ignore
//! use verum_vbc::serialize::{serialize_module, serialize_module_compressed};
//! use verum_vbc::compression::CompressionOptions;
//!
//! // Serialize without compression (default)
//! let bytes = serialize_module(&module)?;
//!
//! // Serialize with zstd compression
//! let compressed = serialize_module_compressed(&module, CompressionOptions::zstd())?;
//! ```

use serde::Serialize;

use crate::compression::{compress, CompressionOptions};
use crate::encoding::*;
use crate::error::VbcResult;
use crate::format::{CompressionAlgorithm, VbcFlags, VbcHeader, HEADER_SIZE, MAGIC, VERSION_MAJOR, VERSION_MINOR};
use crate::module::{
    Constant, FfiLibrary, FfiStructLayout, FfiSymbol, FunctionDescriptor, SourceMap,
    SpecializationEntry, VbcModule,
};
use crate::types::{FieldDescriptor, TypeDescriptor, TypeParamDescriptor, TypeRef, VariantDescriptor};

/// Bundle for serializing FFI tables together.
/// This groups libraries, symbols, and layouts for efficient serialization.
#[derive(Serialize)]
struct FfiBundle<'a> {
    libraries: &'a [FfiLibrary],
    symbols: &'a [FfiSymbol],
    layouts: &'a [FfiStructLayout],
}

/// Serializes a VBC module to binary format without compression.
pub fn serialize_module(module: &VbcModule) -> VbcResult<Vec<u8>> {
    serialize_module_compressed(module, CompressionOptions::none())
}

/// Serializes a VBC module to binary format with optional compression.
///
/// The bytecode section is compressed using the specified algorithm if it
/// meets the size threshold. Other sections (string table, extensions) are
/// not compressed as they're typically smaller and already compact.
///
/// # Arguments
/// * `module` - The VBC module to serialize
/// * `options` - Compression options (algorithm, level, threshold)
///
/// # Example
/// ```rust,ignore
/// use verum_vbc::compression::CompressionOptions;
/// use verum_vbc::serialize::serialize_module_compressed;
///
/// // With zstd compression (default settings)
/// let bytes = serialize_module_compressed(&module, CompressionOptions::zstd())?;
///
/// // With lz4 for faster decompression
/// let bytes = serialize_module_compressed(&module, CompressionOptions::lz4())?;
/// ```
pub fn serialize_module_compressed(module: &VbcModule, options: CompressionOptions) -> VbcResult<Vec<u8>> {
    let mut serializer = Serializer::new(options);
    serializer.serialize(module)?;
    Ok(serializer.finish())
}

/// VBC module serializer.
struct Serializer {
    /// Main output buffer.
    output: Vec<u8>,
    /// Compression options.
    compression: CompressionOptions,
    /// Compression algorithm actually used (may be None if data didn't compress well).
    used_algorithm: Option<CompressionAlgorithm>,
}

impl Serializer {
    /// Creates a new serializer with compression options.
    fn new(compression: CompressionOptions) -> Self {
        Self {
            output: Vec::with_capacity(64 * 1024), // 64KB initial capacity
            compression,
            used_algorithm: None,
        }
    }

    /// Serializes a complete module.
    fn serialize(&mut self, module: &VbcModule) -> VbcResult<()> {
        // Reserve space for header
        self.output.resize(HEADER_SIZE, 0);

        // 1. Serialize string table
        let string_table_offset = self.output.len() as u32;
        let string_table_size = self.serialize_string_table(module)?;

        // 2. Serialize type table
        let type_table_offset = self.output.len() as u32;
        let type_table_count = module.types.len() as u32;
        for type_desc in &module.types {
            self.serialize_type_descriptor(type_desc)?;
        }

        // 3. Serialize function table
        let function_table_offset = self.output.len() as u32;
        let function_table_count = module.functions.len() as u32;
        for func_desc in &module.functions {
            self.serialize_function_descriptor(func_desc)?;
        }

        // 4. Serialize constant pool
        let constant_pool_offset = self.output.len() as u32;
        let constant_pool_count = module.constants.len() as u32;
        for constant in &module.constants {
            self.serialize_constant(constant)?;
        }

        // 5. Serialize bytecode (with optional compression)
        let bytecode_offset = self.output.len() as u32;
        let (bytecode_size, _uncompressed_bytecode_size) = self.serialize_bytecode(&module.bytecode)?;

        // 6. Serialize specialization table
        let specialization_table_offset = self.output.len() as u32;
        let specialization_table_count = module.specializations.len() as u32;
        for spec in &module.specializations {
            self.serialize_specialization(spec)?;
        }

        // 7. Serialize source map (optional)
        let (source_map_offset, source_map_size) = if let Some(ref source_map) = module.source_map {
            let offset = self.output.len() as u32;
            let size = self.serialize_source_map(source_map)?;
            (offset, size)
        } else {
            (0, 0)
        };

        // 8. Serialize extensions (tensor metadata) if present
        let (extensions_offset, extensions_size, extra_flags) =
            self.serialize_extensions(module)?;

        // Compute content hash using blake3 (truncated to u64 for header).
        // blake3::Hash::as_bytes() always returns a 32-byte buffer; `[..8]`
        // on it is statically safe.  We use `expect` rather than
        // `unwrap_or([0u8; 8])` because the silent-zero fallback would
        // DEFEAT the integrity check entirely — every module would write
        // a zero hash and every verify-side recompute would match it,
        // making tampering undetectable.  Panic-on-impossible is
        // architecturally correct; an all-zero hash is worse than a
        // crash.
        let content_hash = {
            let hash = blake3::hash(&self.output[HEADER_SIZE..]);
            u64::from_le_bytes(
                hash.as_bytes()[..8]
                    .try_into()
                    .expect("blake3 always returns 32 bytes; [..8] always fits"),
            )
        };

        // Compute dependency hash using blake3 — same invariant.
        let mut dep_data = Vec::new();
        for dep in &module.dependencies {
            encode_u64(dep.hash, &mut dep_data);
        }
        let dependency_hash = {
            let hash = blake3::hash(&dep_data);
            u64::from_le_bytes(
                hash.as_bytes()[..8]
                    .try_into()
                    .expect("blake3 always returns 32 bytes; [..8] always fits"),
            )
        };

        // Build compression flag based on whether compression was used
        let compression_flag = if self.used_algorithm.is_some() {
            VbcFlags::COMPRESSED
        } else {
            VbcFlags::empty()
        };

        // Build and write header
        let header = VbcHeader {
            magic: MAGIC,
            version_major: VERSION_MAJOR,
            version_minor: VERSION_MINOR,
            flags: module.header.flags | extra_flags | compression_flag,
            module_name_offset: module.strings.iter()
                .find(|(s, _)| *s == module.name)
                .map(|(_, id)| id.0)
                .unwrap_or(0),
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
        };

        self.write_header(&header);

        Ok(())
    }

    /// Writes the header at the beginning of the output.
    fn write_header(&mut self, header: &VbcHeader) {
        let mut buf = Vec::with_capacity(HEADER_SIZE);

        buf.extend_from_slice(&header.magic);
        encode_u16(header.version_major, &mut buf);
        encode_u16(header.version_minor, &mut buf);
        encode_u32(header.flags.bits(), &mut buf);
        encode_u32(header.module_name_offset, &mut buf);
        encode_u32(header.type_table_offset, &mut buf);
        encode_u32(header.type_table_count, &mut buf);
        encode_u32(header.function_table_offset, &mut buf);
        encode_u32(header.function_table_count, &mut buf);
        encode_u32(header.constant_pool_offset, &mut buf);
        encode_u32(header.constant_pool_count, &mut buf);
        encode_u32(header.string_table_offset, &mut buf);
        encode_u32(header.string_table_size, &mut buf);
        encode_u32(header.bytecode_offset, &mut buf);
        encode_u32(header.bytecode_size, &mut buf);
        encode_u32(header.specialization_table_offset, &mut buf);
        encode_u32(header.specialization_table_count, &mut buf);
        encode_u32(header.source_map_offset, &mut buf);
        encode_u32(header.source_map_size, &mut buf);
        encode_u64(header.content_hash, &mut buf);
        encode_u64(header.dependency_hash, &mut buf);
        encode_u32(header.extensions_offset, &mut buf);
        encode_u32(header.extensions_size, &mut buf);

        debug_assert_eq!(buf.len(), HEADER_SIZE);
        self.output[..HEADER_SIZE].copy_from_slice(&buf);
    }

    /// Serializes the string table.
    fn serialize_string_table(&mut self, module: &VbcModule) -> VbcResult<u32> {
        let start = self.output.len();

        // Write each string: u32 length + UTF-8 bytes
        for (s, _) in module.strings.iter() {
            let bytes = s.as_bytes();
            encode_u32(bytes.len() as u32, &mut self.output);
            self.output.extend_from_slice(bytes);
        }

        Ok((self.output.len() - start) as u32)
    }

    /// Serializes bytecode with optional compression.
    ///
    /// # Format
    ///
    /// The bytecode section has a compression header followed by the data:
    /// - `u8`: Compression algorithm (0=None, 1=Zstd, 2=Lz4)
    /// - `u32`: Uncompressed size (only present if algorithm != None)
    /// - `bytes`: Compressed or uncompressed bytecode
    ///
    /// # Returns
    ///
    /// Returns `(stored_size, uncompressed_size)` where stored_size is what
    /// was actually written (including compression header).
    fn serialize_bytecode(&mut self, bytecode: &[u8]) -> VbcResult<(u32, u32)> {
        let uncompressed_size = bytecode.len() as u32;

        // Try to compress if compression is enabled
        if let Some((compressed, algorithm)) = compress(bytecode, &self.compression)? {
            // Compression succeeded and reduced size
            self.used_algorithm = Some(algorithm);

            // Write compression header
            self.output.push(algorithm as u8);
            encode_u32(uncompressed_size, &mut self.output);

            // Write compressed data
            let compressed_len = compressed.len() as u32;
            self.output.extend_from_slice(&compressed);

            // Return stored size (header + compressed data)
            // 1 byte algorithm + 4 bytes uncompressed_size + compressed_len
            Ok((1 + 4 + compressed_len, uncompressed_size))
        } else {
            // No compression - write raw data with None algorithm marker
            self.output.push(CompressionAlgorithm::None as u8);
            self.output.extend_from_slice(bytecode);

            // Return stored size (1 byte header + uncompressed data)
            Ok((1 + uncompressed_size, uncompressed_size))
        }
    }

    /// Serializes a type descriptor.
    fn serialize_type_descriptor(&mut self, desc: &TypeDescriptor) -> VbcResult<()> {
        encode_u32(desc.id.0, &mut self.output);
        encode_u32(desc.name.0, &mut self.output);
        self.output.push(desc.kind as u8);
        self.output.push(desc.visibility as u8);
        encode_u32(desc.size, &mut self.output);
        encode_u32(desc.alignment, &mut self.output);

        // Type parameters
        encode_varint(desc.type_params.len() as u64, &mut self.output);
        for param in &desc.type_params {
            self.serialize_type_param(param)?;
        }

        // Fields
        encode_varint(desc.fields.len() as u64, &mut self.output);
        for field in &desc.fields {
            self.serialize_field(field)?;
        }

        // Variants
        encode_varint(desc.variants.len() as u64, &mut self.output);
        for variant in &desc.variants {
            self.serialize_variant(variant)?;
        }

        // Drop/clone functions
        self.serialize_optional_u32(desc.drop_fn);
        self.serialize_optional_u32(desc.clone_fn);

        // Protocols
        encode_varint(desc.protocols.len() as u64, &mut self.output);
        for proto_impl in &desc.protocols {
            encode_u32(proto_impl.protocol.0, &mut self.output);
            encode_varint(proto_impl.methods.len() as u64, &mut self.output);
            for &method in &proto_impl.methods {
                encode_u32(method, &mut self.output);
            }
        }

        Ok(())
    }

    /// Serializes a type parameter descriptor.
    fn serialize_type_param(&mut self, param: &TypeParamDescriptor) -> VbcResult<()> {
        encode_u32(param.name.0, &mut self.output);
        encode_u16(param.id.0, &mut self.output);
        self.output.push(param.variance as u8);

        // Bounds
        encode_varint(param.bounds.len() as u64, &mut self.output);
        for bound in &param.bounds {
            encode_u32(bound.0, &mut self.output);
        }

        // Default type
        if let Some(ref default) = param.default {
            self.output.push(1);
            self.serialize_type_ref(default)?;
        } else {
            self.output.push(0);
        }

        Ok(())
    }

    /// Serializes a field descriptor.
    fn serialize_field(&mut self, field: &FieldDescriptor) -> VbcResult<()> {
        encode_u32(field.name.0, &mut self.output);
        self.serialize_type_ref(&field.type_ref)?;
        encode_u32(field.offset, &mut self.output);
        self.output.push(field.visibility as u8);
        Ok(())
    }

    /// Serializes a variant descriptor.
    fn serialize_variant(&mut self, variant: &VariantDescriptor) -> VbcResult<()> {
        encode_u32(variant.name.0, &mut self.output);
        encode_u32(variant.tag, &mut self.output);
        self.output.push(variant.kind as u8);
        self.output.push(variant.arity);

        // Payload type
        if let Some(ref payload) = variant.payload {
            self.output.push(1);
            self.serialize_type_ref(payload)?;
        } else {
            self.output.push(0);
        }

        // Fields (for record variants)
        encode_varint(variant.fields.len() as u64, &mut self.output);
        for field in &variant.fields {
            self.serialize_field(field)?;
        }

        Ok(())
    }

    /// Serializes a type reference.
    fn serialize_type_ref(&mut self, type_ref: &TypeRef) -> VbcResult<()> {
        match type_ref {
            TypeRef::Concrete(id) => {
                self.output.push(0x01);
                encode_u32(id.0, &mut self.output);
            }
            TypeRef::Generic(param_id) => {
                self.output.push(0x02);
                encode_u16(param_id.0, &mut self.output);
            }
            TypeRef::Instantiated { base, args } => {
                self.output.push(0x03);
                encode_u32(base.0, &mut self.output);
                encode_varint(args.len() as u64, &mut self.output);
                for arg in args {
                    self.serialize_type_ref(arg)?;
                }
            }
            TypeRef::Function {
                params,
                return_type,
                contexts,
            } => {
                self.output.push(0x04);
                encode_varint(params.len() as u64, &mut self.output);
                for param in params {
                    self.serialize_type_ref(param)?;
                }
                self.serialize_type_ref(return_type)?;
                encode_varint(contexts.len() as u64, &mut self.output);
                for ctx in contexts {
                    encode_u32(ctx.0, &mut self.output);
                }
            }
            TypeRef::Reference {
                inner,
                mutability,
                tier,
            } => {
                let tag = 0x05 | ((*mutability as u8) << 4) | ((*tier as u8) << 6);
                self.output.push(tag);
                self.serialize_type_ref(inner)?;
            }
            TypeRef::Tuple(elems) => {
                self.output.push(0x06);
                encode_varint(elems.len() as u64, &mut self.output);
                for elem in elems {
                    self.serialize_type_ref(elem)?;
                }
            }
            TypeRef::Array { element, length } => {
                self.output.push(0x07);
                self.serialize_type_ref(element)?;
                encode_u64(*length, &mut self.output);
            }
            TypeRef::Slice(inner) => {
                self.output.push(0x08);
                self.serialize_type_ref(inner)?;
            }
            TypeRef::Rank2Function {
                type_param_count,
                params,
                return_type,
                contexts,
            } => {
                self.output.push(0x09);
                encode_u16(*type_param_count, &mut self.output);
                encode_varint(params.len() as u64, &mut self.output);
                for param in params {
                    self.serialize_type_ref(param)?;
                }
                self.serialize_type_ref(return_type)?;
                encode_varint(contexts.len() as u64, &mut self.output);
                for ctx in contexts {
                    encode_u32(ctx.0, &mut self.output);
                }
            }
        }
        Ok(())
    }

    /// Serializes a function descriptor.
    fn serialize_function_descriptor(&mut self, desc: &FunctionDescriptor) -> VbcResult<()> {
        encode_u32(desc.id.0, &mut self.output);
        encode_u32(desc.name.0, &mut self.output);
        self.serialize_optional_u32(desc.parent_type.map(|t| t.0));
        self.output.push(desc.visibility as u8);

        // Flags
        let flags = (desc.is_inline_candidate as u8)
            | ((desc.is_generic as u8) << 1)
            | ((desc.is_generator as u8) << 2);
        self.output.push(flags);

        // Properties
        encode_u16(desc.properties.bits(), &mut self.output);

        // Bytecode location
        encode_u32(desc.bytecode_offset, &mut self.output);
        encode_u32(desc.bytecode_length, &mut self.output);
        encode_u16(desc.locals_count, &mut self.output);
        encode_u16(desc.register_count, &mut self.output);
        encode_u16(desc.max_stack, &mut self.output);

        // Type parameters
        encode_varint(desc.type_params.len() as u64, &mut self.output);
        for param in &desc.type_params {
            self.serialize_type_param(param)?;
        }

        // Parameters
        encode_varint(desc.params.len() as u64, &mut self.output);
        for param in &desc.params {
            encode_u32(param.name.0, &mut self.output);
            self.serialize_type_ref(&param.type_ref)?;
            self.output.push(param.is_mut as u8);
            self.serialize_optional_u32(param.default.map(|c| c.0));
        }

        // Return type
        self.serialize_type_ref(&desc.return_type)?;

        // Contexts
        encode_varint(desc.contexts.len() as u64, &mut self.output);
        for ctx in &desc.contexts {
            encode_u32(ctx.0, &mut self.output);
        }

        // Generator metadata (only if is_generator)
        if desc.is_generator {
            // has_yield_type flag
            self.output.push(desc.yield_type.is_some() as u8);
            if let Some(ref yield_type) = desc.yield_type {
                self.serialize_type_ref(yield_type)?;
            }
            encode_u16(desc.suspend_point_count, &mut self.output);
        }

        Ok(())
    }

    /// Serializes a constant.
    fn serialize_constant(&mut self, constant: &Constant) -> VbcResult<()> {
        self.output.push(constant.tag());
        match constant {
            Constant::Int(v) => encode_i64(*v, &mut self.output),
            Constant::Float(v) => encode_f64(*v, &mut self.output),
            Constant::String(id) => encode_u32(id.0, &mut self.output),
            Constant::Type(type_ref) => self.serialize_type_ref(type_ref)?,
            Constant::Function(id) => encode_u32(id.0, &mut self.output),
            Constant::Protocol(id) => encode_u32(id.0, &mut self.output),
            Constant::Array(items) => {
                encode_varint(items.len() as u64, &mut self.output);
                for item in items {
                    encode_u32(item.0, &mut self.output);
                }
            }
            Constant::Bytes(bytes) => {
                encode_varint(bytes.len() as u64, &mut self.output);
                self.output.extend_from_slice(bytes);
            }
        }
        Ok(())
    }

    /// Serializes a specialization entry.
    fn serialize_specialization(&mut self, spec: &SpecializationEntry) -> VbcResult<()> {
        encode_u32(spec.generic_fn.0, &mut self.output);
        encode_u64(spec.hash, &mut self.output);
        encode_u32(spec.bytecode_offset, &mut self.output);
        encode_u32(spec.bytecode_length, &mut self.output);
        encode_u16(spec.register_count, &mut self.output);

        encode_varint(spec.type_args.len() as u64, &mut self.output);
        for arg in &spec.type_args {
            self.serialize_type_ref(arg)?;
        }

        Ok(())
    }

    /// Serializes a source map.
    fn serialize_source_map(&mut self, source_map: &SourceMap) -> VbcResult<u32> {
        let start = self.output.len();

        // Files
        encode_varint(source_map.files.len() as u64, &mut self.output);
        for file in &source_map.files {
            encode_u32(file.0, &mut self.output);
        }

        // Entries
        encode_varint(source_map.entries.len() as u64, &mut self.output);
        for entry in &source_map.entries {
            encode_u32(entry.bytecode_offset, &mut self.output);
            encode_u16(entry.file_idx, &mut self.output);
            encode_u32(entry.line, &mut self.output);
            encode_u16(entry.column, &mut self.output);
        }

        Ok((self.output.len() - start) as u32)
    }

    /// Serializes the extensions section (tensor metadata, FFI, dependencies).
    ///
    /// Returns (offset, size, extra_flags) where extra_flags are VbcFlags
    /// indicating which tensor metadata is present.
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
    fn serialize_extensions(&mut self, module: &VbcModule) -> VbcResult<(u32, u32, VbcFlags)> {
        // Check if any tensor metadata is present
        let has_shapes = !module.shape_metadata.is_empty();
        let has_device_hints = !module.device_hints.is_empty();
        let has_distribution = !module.distribution.is_empty();
        let has_autodiff = !module.autodiff_graph.is_empty();
        let has_mlir_hints = !module.mlir_hints.is_empty();
        let has_ffi = !module.ffi_libraries.is_empty()
            || !module.ffi_symbols.is_empty()
            || !module.ffi_layouts.is_empty();
        let has_dependencies = !module.dependencies.is_empty();

        if !has_shapes
            && !has_device_hints
            && !has_distribution
            && !has_autodiff
            && !has_mlir_hints
            && !has_ffi
            && !has_dependencies
        {
            // No extensions, return zeros
            return Ok((0, 0, VbcFlags::empty()));
        }

        let start = self.output.len();
        let mut extra_flags = VbcFlags::empty();

        // Extension section header: bitmask of present sections
        let mut section_mask: u8 = 0;
        if has_shapes {
            section_mask |= 0x01;
            extra_flags |= VbcFlags::HAS_TENSORS;
        }
        if has_device_hints {
            section_mask |= 0x02;
            extra_flags |= VbcFlags::HAS_GPU;
        }
        if has_distribution {
            section_mask |= 0x04;
        }
        if has_autodiff {
            section_mask |= 0x08;
            extra_flags |= VbcFlags::HAS_AUTODIFF;
        }
        if has_mlir_hints {
            section_mask |= 0x10;
        }
        if has_ffi {
            section_mask |= 0x20;
            // HAS_FFI flag is already set based on ffi_symbols in update_flags()
        }
        if has_dependencies {
            section_mask |= 0x40;
        }
        self.output.push(section_mask);

        // Serialize each present section using bincode
        // Each section: u32 length + bincode data
        if has_shapes {
            let data =
                bincode::serialize(&module.shape_metadata).map_err(|e| {
                    crate::error::VbcError::Serialization(format!("shape_metadata: {}", e))
                })?;
            encode_u32(data.len() as u32, &mut self.output);
            self.output.extend_from_slice(&data);
        }

        if has_device_hints {
            let data =
                bincode::serialize(&module.device_hints).map_err(|e| {
                    crate::error::VbcError::Serialization(format!("device_hints: {}", e))
                })?;
            encode_u32(data.len() as u32, &mut self.output);
            self.output.extend_from_slice(&data);
        }

        if has_distribution {
            let data =
                bincode::serialize(&module.distribution).map_err(|e| {
                    crate::error::VbcError::Serialization(format!("distribution: {}", e))
                })?;
            encode_u32(data.len() as u32, &mut self.output);
            self.output.extend_from_slice(&data);
        }

        if has_autodiff {
            let data =
                bincode::serialize(&module.autodiff_graph).map_err(|e| {
                    crate::error::VbcError::Serialization(format!("autodiff_graph: {}", e))
                })?;
            encode_u32(data.len() as u32, &mut self.output);
            self.output.extend_from_slice(&data);
        }

        if has_mlir_hints {
            let data =
                bincode::serialize(&module.mlir_hints).map_err(|e| {
                    crate::error::VbcError::Serialization(format!("mlir_hints: {}", e))
                })?;
            encode_u32(data.len() as u32, &mut self.output);
            self.output.extend_from_slice(&data);
        }

        // FFI tables: libraries, symbols, layouts bundled together
        if has_ffi {
            let ffi_bundle = FfiBundle {
                libraries: &module.ffi_libraries,
                symbols: &module.ffi_symbols,
                layouts: &module.ffi_layouts,
            };
            let data = bincode::serialize(&ffi_bundle).map_err(|e| {
                crate::error::VbcError::Serialization(format!("ffi_tables: {}", e))
            })?;
            encode_u32(data.len() as u32, &mut self.output);
            self.output.extend_from_slice(&data);
        }

        // Dependencies
        if has_dependencies {
            let data = bincode::serialize(&module.dependencies).map_err(|e| {
                crate::error::VbcError::Serialization(format!("dependencies: {}", e))
            })?;
            encode_u32(data.len() as u32, &mut self.output);
            self.output.extend_from_slice(&data);
        }

        Ok((start as u32, (self.output.len() - start) as u32, extra_flags))
    }

    /// Serializes an optional u32.
    fn serialize_optional_u32(&mut self, value: Option<u32>) {
        if let Some(v) = value {
            self.output.push(1);
            encode_u32(v, &mut self.output);
        } else {
            self.output.push(0);
        }
    }

    /// Finishes serialization and returns the output buffer.
    fn finish(self) -> Vec<u8> {
        self.output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_empty_module() {
        let module = VbcModule::new("test".to_string());
        let bytes = serialize_module(&module).unwrap();

        // Check header
        assert_eq!(&bytes[0..4], MAGIC);
        assert!(bytes.len() >= HEADER_SIZE);
    }

    #[test]
    fn test_serialize_with_strings() {
        let mut module = VbcModule::new("test_module".to_string());
        module.intern_string("hello");
        module.intern_string("world");

        let bytes = serialize_module(&module).unwrap();
        assert!(bytes.len() > HEADER_SIZE);
    }

    #[test]
    fn test_serialize_with_constant() {
        let mut module = VbcModule::new("test".to_string());
        module.add_constant(Constant::Int(42));
        module.add_constant(Constant::Float(3.14));

        let bytes = serialize_module(&module).unwrap();
        assert!(bytes.len() > HEADER_SIZE);
    }

    #[test]
    fn test_serialize_with_tensor_metadata() {
        use crate::deserialize::deserialize_module;
        use crate::metadata::{
            shape::{DType, InstructionId, ShapeDim, StaticShape, SymbolDef},
            device::{BlockId, DevicePreference, ValueId},
            distribution::{MeshTopology, ShardingSpec},
            autodiff::{CheckpointBoundary, VjpRule},
            mlir::{FusionGroup, RegionId},
        };

        let mut module = VbcModule::new("tensor_test".to_string());

        // Add shape metadata
        let batch_symbol = module.shape_metadata.define_symbol(
            SymbolDef::new("batch_size").with_bounds(1, 256)
        );
        module.shape_metadata.add_static_shape(
            InstructionId(0),
            StaticShape::new(
                vec![ShapeDim::Symbolic(batch_symbol), ShapeDim::Static(1024)],
                DType::F32,
            ),
        );

        // Add device hints
        module.device_hints.set_placement(BlockId(0), DevicePreference::cuda());
        module.device_hints.set_placement(BlockId(1), DevicePreference::CPU);

        // Add distribution metadata
        module.distribution.set_mesh(MeshTopology::grid(4, 8));
        module.distribution.add_sharding(
            ValueId(0),
            ShardingSpec::shard_dim(2, 0, "hosts"),
        );

        // Add autodiff graph
        module.autodiff_graph.add_vjp(VjpRule::new(
            crate::FunctionId(0),
            crate::FunctionId(1),
        ));
        module.autodiff_graph.add_checkpoint(
            CheckpointBoundary::new(InstructionId(10), InstructionId(50))
                .with_memory_savings(1024 * 1024),
        );

        // Add MLIR hints
        module.mlir_hints.add_mlir_region(RegionId(0));
        module.mlir_hints.add_fusion_group(
            FusionGroup::new(
                0,
                vec![InstructionId(0), InstructionId(1), InstructionId(2)],
            )
            .with_pattern("matmul_bias")
            .with_speedup(1.5),
        );

        // Serialize
        let bytes = serialize_module(&module).unwrap();

        // Deserialize
        let loaded = deserialize_module(&bytes).unwrap();

        // Verify shape metadata
        assert!(!loaded.shape_metadata.is_empty());
        assert!(loaded.shape_metadata.get_shape(InstructionId(0)).is_some());
        let shape = loaded.shape_metadata.get_shape(InstructionId(0)).unwrap();
        assert_eq!(shape.ndim(), 2);
        assert_eq!(shape.dtype, DType::F32);

        // Verify device hints
        assert!(!loaded.device_hints.is_empty());
        assert!(matches!(
            loaded.device_hints.get_placement(BlockId(0)),
            DevicePreference::GPU { .. }
        ));
        assert!(matches!(
            loaded.device_hints.get_placement(BlockId(1)),
            DevicePreference::CPU
        ));

        // Verify distribution
        assert!(!loaded.distribution.is_empty());
        assert!(loaded.distribution.is_distributed());
        assert!(loaded.distribution.get_sharding(ValueId(0)).is_some());

        // Verify autodiff
        assert!(!loaded.autodiff_graph.is_empty());
        assert!(loaded.autodiff_graph.get_vjp(crate::FunctionId(0)).is_some());
        assert_eq!(loaded.autodiff_graph.checkpoints.len(), 1);
        assert_eq!(loaded.autodiff_graph.checkpoint_memory_savings(), 1024 * 1024);

        // Verify MLIR hints
        assert!(!loaded.mlir_hints.is_empty());
        assert_eq!(loaded.mlir_hints.mlir_regions.len(), 1);
        assert_eq!(loaded.mlir_hints.fusion_groups.len(), 1);
        let fusion_group = &loaded.mlir_hints.fusion_groups[0];
        assert_eq!(fusion_group.pattern, Some("matmul_bias".to_string()));
        assert_eq!(fusion_group.estimated_speedup, 1.5);

        // Verify flags are set correctly
        assert!(loaded.header.flags.contains(VbcFlags::HAS_TENSORS));
        assert!(loaded.header.flags.contains(VbcFlags::HAS_GPU));
        assert!(loaded.header.flags.contains(VbcFlags::HAS_AUTODIFF));
    }

    #[test]
    fn test_serialize_with_ffi_and_dependencies() {
        use crate::deserialize::deserialize_module;
        use crate::module::{
            CType, FfiLibrary, FfiPlatform, FfiSignature, FfiStructField, FfiStructLayout,
            FfiSymbol, MemoryEffects, ModuleDependency,
        };
        use smallvec::smallvec;

        let mut module = VbcModule::new("ffi_test".to_string());

        // Add FFI library
        let lib_name = module.intern_string("libSystem.B.dylib");
        module.ffi_libraries.push(FfiLibrary {
            name: lib_name,
            platform: FfiPlatform::Darwin,
            required: true,
            version: None,
        });

        // Add second library for Windows
        let win_lib_name = module.intern_string("kernel32.dll");
        let win_version = module.intern_string("10.0");
        module.ffi_libraries.push(FfiLibrary {
            name: win_lib_name,
            platform: FfiPlatform::Windows,
            required: false,
            version: Some(win_version),
        });

        // Add FFI symbol (getpid)
        let getpid_name = module.intern_string("getpid");
        module.ffi_symbols.push(FfiSymbol::new(
            getpid_name,
            FfiSignature::new(CType::I32, smallvec![]),
        ));

        // Add FFI symbol (malloc)
        let malloc_name = module.intern_string("malloc");
        let mut malloc_sym = FfiSymbol::new(
            malloc_name,
            FfiSignature::new(CType::Ptr, smallvec![CType::Size]),
        );
        malloc_sym.memory_effects = MemoryEffects::ALLOCS;
        module.ffi_symbols.push(malloc_sym);

        // Add FFI symbol with variadic signature (printf)
        let printf_name = module.intern_string("printf");
        let mut printf_sig = FfiSignature::new(CType::I32, smallvec![CType::CStr]);
        printf_sig.is_variadic = true;
        printf_sig.fixed_param_count = 1;
        let mut printf_sym = FfiSymbol::new(printf_name, printf_sig);
        printf_sym.memory_effects = MemoryEffects::IO;
        module.ffi_symbols.push(printf_sym);

        // Add FFI struct layout
        let point_name = module.intern_string("Point");
        let x_name = module.intern_string("x");
        let y_name = module.intern_string("y");
        module.ffi_layouts.push(FfiStructLayout {
            name: point_name,
            size: 16,
            align: 8,
            fields: vec![
                FfiStructField {
                    name: x_name,
                    c_type: CType::F64,
                    offset: 0,
                    size: 8,
                    align: 8,
                },
                FfiStructField {
                    name: y_name,
                    c_type: CType::F64,
                    offset: 8,
                    size: 8,
                    align: 8,
                },
            ],
            verum_type: None,
        });

        // Add dependencies
        let dep1_name = module.intern_string("std.core");
        module.dependencies.push(ModuleDependency {
            name: dep1_name,
            hash: 0x1234567890abcdef,
        });
        let dep2_name = module.intern_string("std.io");
        module.dependencies.push(ModuleDependency {
            name: dep2_name,
            hash: 0xfedcba0987654321,
        });

        // Update flags
        module.update_flags();

        // Serialize
        let bytes = serialize_module(&module).unwrap();

        // Deserialize
        let loaded = deserialize_module(&bytes).unwrap();

        // Verify FFI libraries
        assert_eq!(loaded.ffi_libraries.len(), 2);
        assert_eq!(loaded.get_string(loaded.ffi_libraries[0].name), Some("libSystem.B.dylib"));
        assert_eq!(loaded.ffi_libraries[0].platform, FfiPlatform::Darwin);
        assert!(loaded.ffi_libraries[0].required);
        assert_eq!(loaded.get_string(loaded.ffi_libraries[1].name), Some("kernel32.dll"));
        assert_eq!(loaded.ffi_libraries[1].platform, FfiPlatform::Windows);
        assert!(!loaded.ffi_libraries[1].required);

        // Verify FFI symbols
        assert_eq!(loaded.ffi_symbols.len(), 3);
        assert_eq!(loaded.get_string(loaded.ffi_symbols[0].name), Some("getpid"));
        assert_eq!(loaded.ffi_symbols[0].signature.return_type, CType::I32);
        assert!(loaded.ffi_symbols[0].signature.param_types.is_empty());

        assert_eq!(loaded.get_string(loaded.ffi_symbols[1].name), Some("malloc"));
        assert_eq!(loaded.ffi_symbols[1].signature.return_type, CType::Ptr);
        assert_eq!(loaded.ffi_symbols[1].signature.param_types.len(), 1);
        assert!(loaded.ffi_symbols[1].memory_effects.contains(MemoryEffects::ALLOCS));

        assert_eq!(loaded.get_string(loaded.ffi_symbols[2].name), Some("printf"));
        assert!(loaded.ffi_symbols[2].signature.is_variadic);
        assert_eq!(loaded.ffi_symbols[2].signature.fixed_param_count, 1);
        assert!(loaded.ffi_symbols[2].memory_effects.contains(MemoryEffects::IO));

        // Verify FFI struct layout
        assert_eq!(loaded.ffi_layouts.len(), 1);
        assert_eq!(loaded.get_string(loaded.ffi_layouts[0].name), Some("Point"));
        assert_eq!(loaded.ffi_layouts[0].size, 16);
        assert_eq!(loaded.ffi_layouts[0].align, 8);
        assert_eq!(loaded.ffi_layouts[0].fields.len(), 2);
        assert_eq!(loaded.get_string(loaded.ffi_layouts[0].fields[0].name), Some("x"));
        assert_eq!(loaded.ffi_layouts[0].fields[0].c_type, CType::F64);
        assert_eq!(loaded.ffi_layouts[0].fields[0].offset, 0);

        // Verify dependencies
        assert_eq!(loaded.dependencies.len(), 2);
        assert_eq!(loaded.get_string(loaded.dependencies[0].name), Some("std.core"));
        assert_eq!(loaded.dependencies[0].hash, 0x1234567890abcdef);
        assert_eq!(loaded.get_string(loaded.dependencies[1].name), Some("std.io"));
        assert_eq!(loaded.dependencies[1].hash, 0xfedcba0987654321);

        // Verify HAS_FFI flag is set
        assert!(loaded.header.flags.contains(VbcFlags::HAS_FFI));
    }

    #[cfg(feature = "compression")]
    #[test]
    fn test_compression_roundtrip_zstd() {
        use crate::compression::CompressionOptions;
        use crate::deserialize::deserialize_module;

        let mut module = VbcModule::new("compression_test".to_string());

        // Add a significant amount of bytecode (compressible data)
        // NOP instructions compress very well
        module.bytecode = vec![0x00; 4096]; // 4KB of zeros (highly compressible)

        // Add some strings and constants
        module.intern_string("hello");
        module.intern_string("world");
        module.add_constant(Constant::Int(42));
        module.add_constant(Constant::Float(3.14159));

        // Serialize with zstd compression
        let compressed_bytes = serialize_module_compressed(&module, CompressionOptions::zstd()).unwrap();

        // Serialize without compression for comparison
        let uncompressed_bytes = serialize_module(&module).unwrap();

        // Compressed should be smaller (zeros compress very well)
        assert!(
            compressed_bytes.len() < uncompressed_bytes.len(),
            "Compressed size ({}) should be less than uncompressed size ({})",
            compressed_bytes.len(),
            uncompressed_bytes.len()
        );

        // Verify COMPRESSED flag is set
        let flags_offset = 8; // After magic (4) + version (4)
        let flags_bits = u32::from_le_bytes(compressed_bytes[flags_offset..flags_offset + 4].try_into().unwrap());
        let flags = VbcFlags::from_bits_truncate(flags_bits);
        assert!(flags.contains(VbcFlags::COMPRESSED), "COMPRESSED flag should be set");

        // Deserialize and verify roundtrip
        let loaded = deserialize_module(&compressed_bytes).unwrap();
        assert_eq!(loaded.name, "compression_test");
        assert_eq!(loaded.bytecode.len(), 4096);
        assert!(loaded.bytecode.iter().all(|&b| b == 0x00));
        assert_eq!(loaded.get_string(crate::types::StringId(0)), Some("compression_test"));
        assert_eq!(loaded.constants.len(), 2);
    }

    #[cfg(feature = "compression")]
    #[test]
    fn test_compression_roundtrip_lz4() {
        use crate::compression::CompressionOptions;
        use crate::deserialize::deserialize_module;

        let mut module = VbcModule::new("lz4_test".to_string());

        // Add compressible bytecode
        module.bytecode = vec![0xAA; 2048]; // Repeated pattern

        // Serialize with lz4 compression
        let compressed_bytes = serialize_module_compressed(&module, CompressionOptions::lz4()).unwrap();

        // Serialize without compression for comparison
        let uncompressed_bytes = serialize_module(&module).unwrap();

        // Compressed should be smaller
        assert!(
            compressed_bytes.len() < uncompressed_bytes.len(),
            "Compressed size ({}) should be less than uncompressed size ({})",
            compressed_bytes.len(),
            uncompressed_bytes.len()
        );

        // Deserialize and verify roundtrip
        let loaded = deserialize_module(&compressed_bytes).unwrap();
        assert_eq!(loaded.name, "lz4_test");
        assert_eq!(loaded.bytecode.len(), 2048);
        assert!(loaded.bytecode.iter().all(|&b| b == 0xAA));
    }

    #[cfg(feature = "compression")]
    #[test]
    fn test_compression_skipped_for_small_data() {
        use crate::compression::CompressionOptions;
        use crate::deserialize::deserialize_module;

        let mut module = VbcModule::new("small_test".to_string());

        // Add small bytecode (below compression threshold)
        module.bytecode = vec![0x00; 100]; // 100 bytes - below 512 threshold

        // Serialize with compression enabled
        let bytes = serialize_module_compressed(&module, CompressionOptions::zstd()).unwrap();

        // COMPRESSED flag should NOT be set (too small to compress)
        let flags_offset = 8;
        let flags_bits = u32::from_le_bytes(bytes[flags_offset..flags_offset + 4].try_into().unwrap());
        let flags = VbcFlags::from_bits_truncate(flags_bits);
        assert!(!flags.contains(VbcFlags::COMPRESSED), "COMPRESSED flag should NOT be set for small data");

        // Roundtrip should still work
        let loaded = deserialize_module(&bytes).unwrap();
        assert_eq!(loaded.bytecode.len(), 100);
    }

    #[test]
    fn test_no_compression_roundtrip() {
        use crate::deserialize::deserialize_module;

        let mut module = VbcModule::new("no_compress_test".to_string());
        module.bytecode = vec![0x55; 1024];

        // Serialize without compression
        let bytes = serialize_module(&module).unwrap();

        // COMPRESSED flag should NOT be set
        let flags_offset = 8;
        let flags_bits = u32::from_le_bytes(bytes[flags_offset..flags_offset + 4].try_into().unwrap());
        let flags = VbcFlags::from_bits_truncate(flags_bits);
        assert!(!flags.contains(VbcFlags::COMPRESSED), "COMPRESSED flag should NOT be set");

        // Roundtrip should work
        let loaded = deserialize_module(&bytes).unwrap();
        assert_eq!(loaded.bytecode.len(), 1024);
        assert!(loaded.bytecode.iter().all(|&b| b == 0x55));
    }
}
