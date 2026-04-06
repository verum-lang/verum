// ARCHITECTURE NOTE: When core/ becomes a proper cog, VBC archive loading should use
// the unified cog dependency system instead of special-case stdlib loading.
//! Stdlib metadata loader from VBC archives.
//!
//! This module provides functionality to load stdlib type metadata from
//! pre-compiled VBC archives (stdlib.vbca). This enables the type checker
//! to use stdlib types without parsing .vr files.
//!
//! Architecture:
//! ```text
//! stdlib.vbca ─── VbcArchive ─── CoreMetadata ─── TypeChecker
//! ```
//!
//! VBC-first pipeline: Source → VBC bytecode → Interpreter (Tier 0) or LLVM AOT (Tier 1).
//! Stdlib bootstrap: loads pre-compiled core/ modules from embedded .vbca archive.

use std::io::Read;
use std::path::Path;
use verum_common::{List, Maybe, OrderedMap, Text};
use verum_types::core_metadata::{
    AssociatedTypeDescriptor, FieldDescriptor, FunctionDescriptor, GenericParam,
    ImplementationDescriptor, MethodSignature, ParamDescriptor, ProtocolDescriptor, ReceiverKind,
    CoreMetadata, TypeDescriptor, TypeDescriptorKind, VariantCase, VariantPayload,
};
use verum_vbc::{
    archive::VbcArchive,
    module::{StringTable, VbcModule},
    types::{PropertySet, TypeDescriptor as VbcTypeDescriptor, TypeKind, TypeRef, VariantDescriptor},
};

/// Helper to safely get a string from the string table with a fallback.
fn get_string(strings: &StringTable, id: verum_vbc::types::StringId) -> Text {
    strings.get(id).unwrap_or("").into()
}

/// Error type for stdlib loading operations.
#[derive(Debug)]
pub enum CoreLoadError {
    /// Archive file not found or unreadable.
    IoError(std::io::Error),
    /// Archive format error.
    ArchiveError(String),
    /// Module deserialization error.
    ModuleError(String),
}

impl std::fmt::Display for CoreLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreLoadError::IoError(e) => write!(f, "IO error: {}", e),
            CoreLoadError::ArchiveError(msg) => write!(f, "Archive error: {}", msg),
            CoreLoadError::ModuleError(msg) => write!(f, "Module error: {}", msg),
        }
    }
}

impl std::error::Error for CoreLoadError {}

impl From<std::io::Error> for CoreLoadError {
    fn from(e: std::io::Error) -> Self {
        CoreLoadError::IoError(e)
    }
}

/// Load stdlib metadata from a VBC archive file.
///
/// This function reads the archive, loads all modules, and converts
/// type information to `CoreMetadata` format.
pub fn load_core_metadata(path: impl AsRef<Path>) -> Result<CoreMetadata, CoreLoadError> {
    let archive = load_archive(path)?;
    convert_archive_to_metadata(&archive)
}

/// Load stdlib metadata from embedded bytes (e.g., from `include_bytes!`).
///
/// This is the preferred method when stdlib.vbca is embedded in the binary.
pub fn load_core_metadata_from_bytes(bytes: &[u8]) -> Result<CoreMetadata, CoreLoadError> {
    let archive = load_archive_from_bytes(bytes)?;
    convert_archive_to_metadata(&archive)
}

/// Load a VBC archive from a file.
pub fn load_archive(path: impl AsRef<Path>) -> Result<VbcArchive, CoreLoadError> {
    let mut file = std::fs::File::open(path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;

    load_archive_from_bytes(&data)
}

/// Load a VBC archive from in-memory bytes.
///
/// This is useful when stdlib.vbca is embedded in the binary.
pub fn load_archive_from_bytes(bytes: &[u8]) -> Result<VbcArchive, CoreLoadError> {
    // Wrap in Cursor for Seek support
    let cursor = std::io::Cursor::new(bytes);
    verum_vbc::read_archive(cursor)
        .map_err(|e| CoreLoadError::ArchiveError(format!("{:?}", e)))
}

/// Convert a VBC archive to CoreMetadata.
///
/// This is the main conversion function that extracts type information
/// from VBC format and converts it to the format expected by TypeChecker.
pub fn convert_archive_to_metadata(
    archive: &VbcArchive,
) -> Result<CoreMetadata, CoreLoadError> {
    let mut metadata = CoreMetadata::default();

    // Set version from archive header (combine major.minor into u32)
    metadata.version = ((archive.header.version_major as u32) << 16)
        | (archive.header.version_minor as u32);

    // Process each module in the archive
    for entry in &archive.index {
        let module = archive
            .load_module(&entry.name)
            .map_err(|e| CoreLoadError::ModuleError(format!("{:?}", e)))?;

        extract_module_metadata(&module, &mut metadata)?;
    }

    Ok(metadata)
}

/// Extract metadata from a single VBC module.
fn extract_module_metadata(
    module: &VbcModule,
    metadata: &mut CoreMetadata,
) -> Result<(), CoreLoadError> {
    let module_path: Text = module.name.clone().into();

    // Extract type definitions
    for vbc_type in &module.types {
        let type_name = get_string(&module.strings, vbc_type.name);

        let descriptor = convert_type_descriptor(vbc_type, &module_path, module)?;

        // Protocols are stored separately
        if matches!(vbc_type.kind, TypeKind::Protocol) {
            let protocol = convert_to_protocol_descriptor(vbc_type, &module_path, module)?;
            metadata.protocols.insert(type_name.clone(), protocol);
        }

        // Avoid overwriting variant types with different variant structures.
        // E.g., base::Ordering (Less|Equal|Greater) vs atomic::Ordering (Relaxed|Acquire|...).
        // When a name collision occurs between two variant types, keep the first one
        // under the short name and store the second under a qualified name only.
        let mut use_qualified = false;
        if matches!(vbc_type.kind, TypeKind::Sum) {
            if let Some(existing) = metadata.types.get(&type_name) {
                if matches!(existing.kind, TypeDescriptorKind::Variant { .. }) {
                    // Both are variants with the same name — keep the first, qualify the second
                    use_qualified = true;
                }
            }
        }
        if use_qualified {
            let qualified_name: Text = format!("{}.{}", module_path, type_name).into();
            metadata.types.insert(qualified_name, descriptor);
        } else {
            metadata.types.insert(type_name, descriptor);
        }
    }

    // Extract function signatures
    for vbc_fn in &module.functions {
        let fn_name = get_string(&module.strings, vbc_fn.name);

        let descriptor = convert_function_descriptor(vbc_fn, &module_path, module)?;
        metadata.functions.insert(fn_name, descriptor);
    }

    // Extract protocol implementations from types
    for vbc_type in &module.types {
        let type_name = get_string(&module.strings, vbc_type.name);
        for proto_impl in &vbc_type.protocols {
            let impl_desc = convert_protocol_impl(proto_impl, &type_name, module)?;
            metadata.implementations.push(impl_desc);
        }
    }

    // Note: VBC SpecializationEntry contains function specializations, not type monomorphizations.
    // Type monomorphizations are computed on-demand by the type checker.
    // The monomorphizations field in CoreMetadata is for pre-computed common instantiations.

    Ok(())
}

/// Convert VBC TypeDescriptor to build_mode TypeDescriptor.
fn convert_type_descriptor(
    vbc_type: &VbcTypeDescriptor,
    module_path: &Text,
    module: &VbcModule,
) -> Result<TypeDescriptor, CoreLoadError> {
    let name = get_string(&module.strings, vbc_type.name);

    let generic_params: List<GenericParam> = vbc_type
        .type_params
        .iter()
        .map(|tp| GenericParam {
            name: get_string(&module.strings, tp.name),
            bounds: tp
                .bounds
                .iter()
                .map(|b| {
                    // Look up protocol name from type table
                    if let Some(proto_type) = module.types.get(b.0 as usize) {
                        get_string(&module.strings, proto_type.name)
                    } else {
                        format!("Protocol{}", b.0).into()
                    }
                })
                .collect(),
            default: tp.default.as_ref().map(|d| type_ref_to_text(d, module)).into(),
        })
        .collect();

    let kind = match vbc_type.kind {
        TypeKind::Record => {
            let fields: List<FieldDescriptor> = vbc_type
                .fields
                .iter()
                .map(|f| FieldDescriptor {
                    name: get_string(&module.strings, f.name),
                    ty: type_ref_to_text(&f.type_ref, module),
                    is_public: f.visibility == verum_vbc::types::Visibility::Public,
                })
                .collect();
            TypeDescriptorKind::Record { fields }
        }
        TypeKind::Sum => {
            let cases: List<VariantCase> = vbc_type
                .variants
                .iter()
                .map(|v| convert_variant_case(v, module))
                .collect();
            TypeDescriptorKind::Variant { cases }
        }
        TypeKind::Protocol => {
            // Will be filled in convert_to_protocol_descriptor
            TypeDescriptorKind::Protocol {
                super_protocols: List::new(),
                associated_types: List::new(),
                required_methods: List::new(),
                default_methods: List::new(),
            }
        }
        TypeKind::Newtype => {
            // Newtype wraps a single field
            if let Some(field) = vbc_type.fields.first() {
                TypeDescriptorKind::Alias {
                    target: type_ref_to_text(&field.type_ref, module),
                }
            } else {
                TypeDescriptorKind::Opaque
            }
        }
        TypeKind::Primitive | TypeKind::Unit | TypeKind::Tuple | TypeKind::Array | TypeKind::Tensor => {
            TypeDescriptorKind::Opaque
        }
    };

    Ok(TypeDescriptor {
        name,
        module_path: module_path.clone(),
        generic_params,
        kind,
        size: if vbc_type.size > 0 {
            Maybe::Some(vbc_type.size as usize)
        } else {
            Maybe::None
        },
        alignment: if vbc_type.alignment > 0 {
            Maybe::Some(vbc_type.alignment as usize)
        } else {
            Maybe::None
        },
        methods: List::new(), // Filled separately
        implements: vbc_type
            .protocols
            .iter()
            .map(|p| {
                if let Some(proto_type) = module.types.get(p.protocol.0 as usize) {
                    get_string(&module.strings, proto_type.name)
                } else {
                    format!("Protocol{}", p.protocol.0).into()
                }
            })
            .collect(),
    })
}

/// Convert VBC TypeDescriptor (Protocol kind) to ProtocolDescriptor.
fn convert_to_protocol_descriptor(
    vbc_type: &VbcTypeDescriptor,
    module_path: &Text,
    module: &VbcModule,
) -> Result<ProtocolDescriptor, CoreLoadError> {
    let name = get_string(&module.strings, vbc_type.name);

    let generic_params: List<GenericParam> = vbc_type
        .type_params
        .iter()
        .map(|tp| GenericParam {
            name: get_string(&module.strings, tp.name),
            bounds: tp
                .bounds
                .iter()
                .map(|b| {
                    if let Some(proto_type) = module.types.get(b.0 as usize) {
                        get_string(&module.strings, proto_type.name)
                    } else {
                        format!("Protocol{}", b.0).into()
                    }
                })
                .collect(),
            default: tp.default.as_ref().map(|d| type_ref_to_text(d, module)).into(),
        })
        .collect();

    // Extract method signatures from protocol's variants
    // In VBC, protocol methods are stored as variants with function type payloads
    let mut required_methods: List<MethodSignature> = List::new();
    let default_methods: List<MethodSignature> = List::new();

    for variant in &vbc_type.variants {
        let method_name = get_string(&module.strings, variant.name);

        // Check if this is a method (has function type payload)
        if let Some(payload_type) = &variant.payload {
            // If the payload is a function type, extract method signature
            if let Some(method_sig) = extract_method_signature_from_type(&method_name, payload_type, module) {
                required_methods.push(method_sig);
            }
        }
    }

    // Super protocols from the protocols[] field on Protocol-kind TypeDescriptors
    let super_protocols: List<Text> = vbc_type.protocols.iter()
        .filter_map(|proto_impl| {
            // For protocol types, protocols[] stores super-protocol references
            let proto_type_id = proto_impl.protocol.0 as usize;
            if let Some(proto_type) = module.types.get(proto_type_id) {
                Some(get_string(&module.strings, proto_type.name))
            } else {
                None
            }
        })
        .collect();

    // Associated types: extract from protocol items that have type-only payloads
    // (Currently protocols encode all items as method variants; associated types
    // will be a future VBC extension — for now return empty list)
    let associated_types: List<AssociatedTypeDescriptor> = List::new();

    Ok(ProtocolDescriptor {
        name,
        module_path: module_path.clone(),
        generic_params,
        super_protocols,
        associated_types,
        required_methods,
        default_methods,
    })
}

/// Extract method signature from a function TypeRef.
fn extract_method_signature_from_type(
    name: &Text,
    type_ref: &TypeRef,
    module: &VbcModule,
) -> Option<MethodSignature> {
    match type_ref {
        TypeRef::Function { params, return_type, .. } => {
            let ret_type = type_ref_to_text(return_type, module);

            // Determine receiver kind based on first parameter
            let receiver = if params.is_empty() {
                ReceiverKind::None
            } else {
                let first_param_ty = type_ref_to_text(&params[0], module);
                if first_param_ty.starts_with("&mut ") && first_param_ty.contains("Self") {
                    ReceiverKind::SelfMut
                } else if first_param_ty.starts_with("&") && first_param_ty.contains("Self") {
                    ReceiverKind::SelfRef
                } else if first_param_ty.as_str() == "Self" {
                    ReceiverKind::SelfValue
                } else {
                    ReceiverKind::None
                }
            };

            // Build params_desc EXCLUDING the self parameter
            // If receiver is not None, skip the first param (self)
            let skip_first = receiver != ReceiverKind::None;
            let params_desc: List<ParamDescriptor> = params
                .iter()
                .enumerate()
                .skip(if skip_first { 1 } else { 0 })
                .map(|(i, ty)| ParamDescriptor {
                    name: format!("arg{}", i).into(),
                    ty: type_ref_to_text(ty, module),
                })
                .collect();

            Some(MethodSignature {
                name: name.clone(),
                receiver,
                params: params_desc,
                return_type: ret_type,
                contexts: List::new(),
                is_async: false,
            })
        }
        _ => None, // Not a function type, skip
    }
}

/// Convert VBC VariantDescriptor to VariantCase.
fn convert_variant_case(variant: &VariantDescriptor, module: &VbcModule) -> VariantCase {
    let name = get_string(&module.strings, variant.name);

    let payload = variant.payload.as_ref().map(|type_ref| {
        // Convert the payload type to the appropriate format
        match type_ref {
            TypeRef::Tuple(elems) => {
                // Tuple variant: extract element types
                let type_names: List<Text> = elems.iter()
                    .map(|t| type_ref_to_text(t, module))
                    .collect();
                VariantPayload::Tuple(type_names)
            }
            _ => {
                // Single type payload - treat as single-element tuple
                let type_name = type_ref_to_text(type_ref, module);
                VariantPayload::Tuple(List::from(vec![type_name]))
            }
        }
    });

    VariantCase {
        name,
        payload: payload.into(),
    }
}

/// Convert VBC FunctionDescriptor to build_mode FunctionDescriptor.
fn convert_function_descriptor(
    vbc_fn: &verum_vbc::module::FunctionDescriptor,
    module_path: &Text,
    module: &VbcModule,
) -> Result<FunctionDescriptor, CoreLoadError> {
    let name = get_string(&module.strings, vbc_fn.name);

    let generic_params: List<GenericParam> = vbc_fn
        .type_params
        .iter()
        .map(|tp| GenericParam {
            name: get_string(&module.strings, tp.name),
            bounds: tp
                .bounds
                .iter()
                .map(|b| {
                    if let Some(proto_type) = module.types.get(b.0 as usize) {
                        get_string(&module.strings, proto_type.name)
                    } else {
                        format!("Protocol{}", b.0).into()
                    }
                })
                .collect(),
            default: tp.default.as_ref().map(|d| type_ref_to_text(d, module)).into(),
        })
        .collect();

    let params: List<ParamDescriptor> = vbc_fn
        .params
        .iter()
        .map(|p| ParamDescriptor {
            name: get_string(&module.strings, p.name),
            ty: type_ref_to_text(&p.type_ref, module),
        })
        .collect();

    let return_type = type_ref_to_text(&vbc_fn.return_type, module);

    let contexts: List<Text> = vbc_fn
        .contexts
        .iter()
        .map(|c| {
            // Resolve context name from the module's context_names table
            if let Some(&name_id) = module.context_names.get(c.0 as usize) {
                get_string(&module.strings, name_id)
            } else {
                format!("Context{}", c.0).into()
            }
        })
        .collect();

    Ok(FunctionDescriptor {
        name,
        module_path: module_path.clone(),
        generic_params,
        params,
        return_type,
        contexts,
        is_async: vbc_fn.properties.contains(PropertySet::ASYNC),
        is_unsafe: false, // Would need VBC metadata for this
        intrinsic_id: Maybe::None,
    })
}

/// Convert VBC ProtocolImpl to ImplementationDescriptor.
fn convert_protocol_impl(
    proto_impl: &verum_vbc::types::ProtocolImpl,
    type_name: &str,
    module: &VbcModule,
) -> Result<ImplementationDescriptor, CoreLoadError> {
    let protocol: Text = if let Some(proto_type) = module.types.get(proto_impl.protocol.0 as usize)
    {
        get_string(&module.strings, proto_type.name)
    } else {
        format!("Protocol{}", proto_impl.protocol.0).into()
    };

    // Extract method names from function IDs
    let methods: List<Text> = proto_impl.methods.iter()
        .filter_map(|&func_id| {
            module.functions.get(func_id as usize)
                .map(|func| get_string(&module.strings, func.name))
        })
        .collect();

    Ok(ImplementationDescriptor {
        protocol,
        target_type: type_name.into(),
        generic_params: List::new(),
        where_clause: List::new(),
        associated_types: OrderedMap::new(),
        methods,
    })
}

/// Convert TypeRef to Text representation.
fn type_ref_to_text(type_ref: &TypeRef, module: &VbcModule) -> Text {
    match type_ref {
        TypeRef::Concrete(id) => {
            if let Some(t) = module.types.get(id.0 as usize) {
                get_string(&module.strings, t.name)
            } else {
                // Built-in type
                match id.0 {
                    0 => "()".into(),
                    1 => "Bool".into(),
                    2 => "Int".into(),
                    3 => "Float".into(),
                    4 => "Text".into(),
                    5 => "!".into(), // Never
                    6 => "U8".into(),
                    7 => "U16".into(),
                    8 => "U32".into(),
                    9 => "U64".into(),
                    10 => "I8".into(),
                    11 => "I16".into(),
                    12 => "I32".into(),
                    13 => "F32".into(),
                    14 => "Ptr".into(),
                    _ => format!("Type{}", id.0).into(),
                }
            }
        }
        TypeRef::Generic(param) => format!("T{}", param.0).into(),
        TypeRef::Instantiated { base, args } => {
            let base_name = if let Some(t) = module.types.get(base.0 as usize) {
                get_string(&module.strings, t.name)
            } else {
                return format!("Type{}", base.0).into();
            };
            let args_str: Vec<String> = args.iter().map(|a| type_ref_to_text(a, module).into()).collect();
            format!("{}<{}>", base_name, args_str.join(", ")).into()
        }
        TypeRef::Function {
            params,
            return_type,
            ..
        } => {
            let params_str: Vec<String> = params.iter().map(|p| type_ref_to_text(p, module).into()).collect();
            let ret_str: String = type_ref_to_text(return_type, module).into();
            format!("fn({}) -> {}", params_str.join(", "), ret_str).into()
        }
        TypeRef::Rank2Function { type_param_count, params, return_type, .. } => {
            let type_params: Vec<String> = (0..*type_param_count).map(|i| format!("R{}", i)).collect();
            let params_str: Vec<String> = params.iter().map(|p| type_ref_to_text(p, module).into()).collect();
            let ret_str: String = type_ref_to_text(return_type, module).into();
            format!("fn<{}>({}) -> {}", type_params.join(", "), params_str.join(", "), ret_str).into()
        }
        TypeRef::Reference { inner, .. } => {
            format!("&{}", type_ref_to_text(inner, module)).into()
        }
        TypeRef::Tuple(elems) => {
            let elems_str: Vec<String> = elems.iter().map(|e| type_ref_to_text(e, module).into()).collect();
            format!("({})", elems_str.join(", ")).into()
        }
        TypeRef::Array { element, length } => {
            format!("[{}; {}]", type_ref_to_text(element, module), length).into()
        }
        TypeRef::Slice(inner) => {
            format!("[{}]", type_ref_to_text(inner, module)).into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_archive_conversion() {
        let archive = VbcArchive::new();
        let result = convert_archive_to_metadata(&archive);
        assert!(result.is_ok());
        let metadata = result.unwrap();
        assert!(metadata.types.is_empty());
    }
}
