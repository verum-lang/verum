//! Type Exporter for VBC Generation
//!
//! This module exports type information from the TypeChecker to VBC format,
//! enabling stdlib to be distributed as VBC with full type metadata.
//!
//! # VBC Type Metadata Format
//!
//! The VBC format includes:
//! - Type definitions (structs, enums, protocols)
//! - Generic type parameters and bounds
//! - Impl blocks with method signatures
//! - Protocol implementations
//! - Exported names and visibility
//!
//! # Usage
//!
//! ```ignore
//! let exporter = TypeExporter::new(&type_checker);
//! let metadata = exporter.export_module("core/maybe")?;
//! vbc_writer.write_type_metadata(&metadata)?;
//! ```

use verum_common::{List, Map, Maybe, Text};
use crate::context::TypeScheme;
use crate::ty::{Type, TypeVar};

/// Exported type definition
#[derive(Debug, Clone)]
pub struct ExportedType {
    /// Type name
    pub name: Text,

    /// Generic type parameters
    pub type_params: List<TypeParam>,

    /// The actual type definition
    pub definition: TypeDefinition,

    /// Whether the type is public
    pub is_public: bool,

    /// Documentation comment
    pub doc: Maybe<Text>,
}

/// Type parameter with optional bounds
#[derive(Debug, Clone)]
pub struct TypeParam {
    /// Parameter name (e.g., "T", "K", "V")
    pub name: Text,

    /// Protocol bounds (e.g., "Eq", "Hash")
    pub bounds: List<Text>,

    /// Default type if any
    pub default: Maybe<Type>,
}

/// Type definition variants
#[derive(Debug, Clone)]
pub enum TypeDefinition {
    /// Record type (struct-like)
    Record {
        fields: List<ExportedField>,
    },

    /// Variant type (enum-like)
    Variant {
        variants: List<ExportedVariant>,
    },

    /// Type alias
    Alias {
        target: Type,
    },

    /// Newtype wrapper
    Newtype {
        inner: Type,
    },

    /// Protocol definition
    Protocol {
        methods: List<ExportedMethod>,
        associated_types: List<AssociatedType>,
    },
}

/// Exported record field
#[derive(Debug, Clone)]
pub struct ExportedField {
    pub name: Text,
    pub ty: Type,
    pub is_public: bool,
}

/// Exported variant
#[derive(Debug, Clone)]
pub struct ExportedVariant {
    pub name: Text,
    pub payload: VariantPayload,
}

/// Variant payload types
#[derive(Debug, Clone)]
pub enum VariantPayload {
    /// Unit variant (no payload)
    Unit,
    /// Tuple variant
    Tuple(List<Type>),
    /// Struct variant
    Struct(List<ExportedField>),
}

/// Exported method signature
#[derive(Debug, Clone)]
pub struct ExportedMethod {
    pub name: Text,
    pub type_params: List<TypeParam>,
    pub params: List<MethodParam>,
    pub return_type: Type,
    pub is_static: bool,
    pub is_async: bool,
}

/// Method parameter
#[derive(Debug, Clone)]
pub struct MethodParam {
    pub name: Text,
    pub ty: Type,
}

/// Associated type in protocol
#[derive(Debug, Clone)]
pub struct AssociatedType {
    pub name: Text,
    pub bounds: List<Text>,
    pub default: Maybe<Type>,
}

/// Exported impl block
#[derive(Debug, Clone)]
pub struct ExportedImpl {
    /// Type being implemented for
    pub for_type: Type,

    /// Protocol being implemented (None for inherent impls)
    pub protocol: Maybe<Text>,

    /// Type parameters
    pub type_params: List<TypeParam>,

    /// Methods in this impl
    pub methods: List<ExportedMethod>,
}

/// Module export metadata
#[derive(Debug, Clone, Default)]
pub struct ModuleExports {
    /// Module path (e.g., "core/maybe")
    pub path: Text,

    /// Exported types
    pub types: List<ExportedType>,

    /// Exported impl blocks
    pub impls: List<ExportedImpl>,

    /// Re-exported items from other modules
    pub re_exports: List<ReExport>,

    /// Module-level functions
    pub functions: List<ExportedMethod>,
}

/// Re-export entry
#[derive(Debug, Clone)]
pub struct ReExport {
    pub local_name: Text,
    pub source_module: Text,
    pub source_name: Text,
}

/// TypeExporter extracts type metadata from TypeChecker
pub struct TypeExporter<'a> {
    /// Reference to inherent methods map
    inherent_methods: &'a Map<Text, Map<Text, TypeScheme>>,

    /// Collected exports
    exports: ModuleExports,
}

impl<'a> TypeExporter<'a> {
    /// Create a new TypeExporter from a TypeChecker's inherent_methods
    pub fn new(inherent_methods: &'a Map<Text, Map<Text, TypeScheme>>) -> Self {
        Self {
            inherent_methods,
            exports: ModuleExports::default(),
        }
    }

    /// Export a parsed module's types
    pub fn export_module(
        &mut self,
        module: &verum_ast::Module,
        module_path: &str,
    ) -> &ModuleExports {
        self.exports.path = Text::from(module_path);

        for item in &module.items {
            match &item.kind {
                verum_ast::ItemKind::Type(type_decl) => {
                    if let Some(exported) = self.export_type_decl(type_decl) {
                        self.exports.types.push(exported);
                    }
                }
                verum_ast::ItemKind::Impl(impl_decl) => {
                    if let Some(exported) = self.export_impl_decl(impl_decl) {
                        self.exports.impls.push(exported);
                    }
                }
                verum_ast::ItemKind::Function(func_decl) => {
                    if let Some(exported) = self.export_function(func_decl) {
                        self.exports.functions.push(exported);
                    }
                }
                _ => {}
            }
        }

        &self.exports
    }

    fn export_type_decl(&self, decl: &verum_ast::TypeDecl) -> Maybe<ExportedType> {
        let name = decl.name.name.clone();
        let is_public = decl.visibility.is_public();

        // Extract type parameters
        let type_params = self.extract_type_params(&decl.generics);

        // Extract type definition
        let definition = self.extract_type_definition(&decl.body);

        Maybe::Some(ExportedType {
            name,
            type_params,
            definition,
            is_public,
            doc: Self::extract_doc_from_attributes(&decl.attributes),
        })
    }

    fn export_impl_decl(&self, decl: &verum_ast::decl::ImplDecl) -> Maybe<ExportedImpl> {
        use verum_ast::decl::ImplKind;

        let type_params = self.extract_type_params(&decl.generics);

        let (for_type, protocol) = match &decl.kind {
            ImplKind::Inherent(ty) => {
                let for_type = self.ast_type_to_type(ty);
                (for_type, Maybe::None)
            }
            ImplKind::Protocol { protocol, for_type, .. } => {
                let for_type_converted = self.ast_type_to_type(for_type);
                let protocol_name = protocol.as_ident()
                    .map(|id| id.name.clone())
                    .unwrap_or_else(|| Text::from("?"));
                (for_type_converted, Maybe::Some(protocol_name))
            }
        };

        let methods = self.extract_impl_methods(decl);

        Maybe::Some(ExportedImpl {
            for_type,
            protocol,
            type_params,
            methods,
        })
    }

    fn export_function(&self, decl: &verum_ast::FunctionDecl) -> Maybe<ExportedMethod> {
        if !decl.visibility.is_public() {
            return Maybe::None;
        }

        let name = decl.name.name.clone();
        let type_params = self.extract_type_params(&decl.generics);
        let params = self.extract_function_params(&decl.params);
        let return_type = decl.return_type.as_ref()
            .map(|ty| self.ast_type_to_type(ty))
            .unwrap_or(Type::Unit);

        Maybe::Some(ExportedMethod {
            name,
            type_params,
            params,
            return_type,
            is_static: true,
            is_async: decl.is_async,
        })
    }

    fn extract_type_params(&self, generics: &[verum_ast::ty::GenericParam]) -> List<TypeParam> {
        generics.iter().filter_map(|param| {
            use verum_ast::ty::GenericParamKind;
            match &param.kind {
                GenericParamKind::Type { name, bounds, default } => {
                    let bounds_list: List<Text> = bounds.iter()
                        .filter_map(|b| {
                            use verum_ast::ty::TypeBoundKind;
                            match &b.kind {
                                TypeBoundKind::Protocol(path) => {
                                    path.as_ident().map(|id| id.name.clone())
                                }
                                _ => None,
                            }
                        })
                        .collect();
                    let default_type = default.as_ref().map(|ty| self.ast_type_to_type(ty));

                    Some(TypeParam {
                        name: name.name.clone(),
                        bounds: bounds_list,
                        default: default_type,
                    })
                }
                _ => None,
            }
        }).collect()
    }

    fn extract_type_definition(&self, body: &verum_ast::decl::TypeDeclBody) -> TypeDefinition {
        use verum_ast::decl::TypeDeclBody;

        match body {
            TypeDeclBody::Record(fields) => {
                let exported_fields = fields.iter().map(|f| {
                    ExportedField {
                        name: f.name.name.clone(),
                        ty: self.ast_type_to_type(&f.ty),
                        is_public: f.visibility.is_public(),
                    }
                }).collect();
                TypeDefinition::Record { fields: exported_fields }
            }

            TypeDeclBody::Variant(variants) => {
                let exported_variants = variants.iter().map(|v| {
                    use verum_ast::decl::VariantData;
                    let payload = match &v.data {
                        Maybe::None => VariantPayload::Unit,
                        Maybe::Some(VariantData::Tuple(types)) => {
                            let tys = types.iter().map(|t| self.ast_type_to_type(t)).collect();
                            VariantPayload::Tuple(tys)
                        }
                        Maybe::Some(VariantData::Record(fields)) => {
                            let fs = fields.iter().map(|f| ExportedField {
                                name: f.name.name.clone(),
                                ty: self.ast_type_to_type(&f.ty),
                                is_public: true,
                            }).collect();
                            VariantPayload::Struct(fs)
                        }
                    };
                    ExportedVariant {
                        name: v.name.name.clone(),
                        payload,
                    }
                }).collect();
                TypeDefinition::Variant { variants: exported_variants }
            }

            TypeDeclBody::Alias(ty) => {
                TypeDefinition::Alias { target: self.ast_type_to_type(ty) }
            }

            TypeDeclBody::Newtype(ty) => {
                TypeDefinition::Newtype { inner: self.ast_type_to_type(ty) }
            }

            TypeDeclBody::Protocol(protocol_body) => {
                use verum_ast::decl::ProtocolItemKind;

                let methods = protocol_body.items.iter().filter_map(|item| {
                    if let ProtocolItemKind::Function { decl: m, .. } = &item.kind {
                        Some(ExportedMethod {
                            name: m.name.name.clone(),
                            type_params: self.extract_type_params(&m.generics),
                            params: self.extract_function_params(&m.params),
                            return_type: m.return_type.as_ref()
                                .map(|t| self.ast_type_to_type(t))
                                .unwrap_or(Type::Unit),
                            is_static: !m.params.first().map(|p| p.is_self()).unwrap_or(false),
                            is_async: m.is_async,
                        })
                    } else {
                        None
                    }
                }).collect();

                let associated_types = protocol_body.items.iter().filter_map(|item| {
                    if let ProtocolItemKind::Type { name, bounds, default_type, .. } = &item.kind {
                        Some(AssociatedType {
                            name: name.name.clone(),
                            bounds: bounds.iter()
                                .filter_map(|b| b.as_ident().map(|id| id.name.clone()))
                                .collect(),
                            default: default_type.as_ref().map(|t| self.ast_type_to_type(t)),
                        })
                    } else {
                        None
                    }
                }).collect();

                TypeDefinition::Protocol { methods, associated_types }
            }

            TypeDeclBody::Tuple(_) | TypeDeclBody::SigmaTuple(_) => {
                TypeDefinition::Record { fields: List::new() }
            }

            TypeDeclBody::Unit => TypeDefinition::Record { fields: List::new() },

            TypeDeclBody::Inductive(_) | TypeDeclBody::Coinductive(_) => {
                TypeDefinition::Record { fields: List::new() }
            }

            // T1-T: quotient types export their base carrier's
            // structure — downstream consumers see `Q` as a refined
            // variant of `T`. Full HIT lowering lives in the
            // elaborator (future follow-up).
            TypeDeclBody::Quotient { .. } => {
                TypeDefinition::Record { fields: List::new() }
            }
        }
    }

    fn extract_impl_methods(&self, decl: &verum_ast::decl::ImplDecl) -> List<ExportedMethod> {
        use verum_ast::decl::ImplItemKind;

        decl.items.iter().filter_map(|item| {
            if let ImplItemKind::Function(func) = &item.kind {
                let is_static = !func.params.first().map(|p| p.is_self()).unwrap_or(false);
                Some(ExportedMethod {
                    name: func.name.name.clone(),
                    type_params: self.extract_type_params(&func.generics),
                    params: self.extract_function_params(&func.params),
                    return_type: func.return_type.as_ref()
                        .map(|t| self.ast_type_to_type(t))
                        .unwrap_or(Type::Unit),
                    is_static,
                    is_async: func.is_async,
                })
            } else {
                None
            }
        }).collect()
    }

    fn extract_function_params(&self, params: &[verum_ast::decl::FunctionParam]) -> List<MethodParam> {
        use verum_ast::decl::FunctionParamKind;

        params.iter().filter_map(|p| {
            match &p.kind {
                FunctionParamKind::Regular { pattern, ty, .. } => {
                    let name = match &pattern.kind {
                        verum_ast::PatternKind::Ident { name, .. } => name.name.clone(),
                        _ => Text::from("_"),
                    };
                    Some(MethodParam {
                        name,
                        ty: self.ast_type_to_type(ty),
                    })
                }
                _ => None, // Skip self parameters
            }
        }).collect()
    }

    /// Extract documentation text from a list of attributes.
    ///
    /// Looks for `@doc("...")` attributes and concatenates their string arguments
    /// into a single doc string. Multiple `@doc` attributes are joined with newlines.
    fn extract_doc_from_attributes(attrs: &[verum_ast::decl::Attribute]) -> Maybe<Text> {
        let mut doc_parts: List<Text> = List::new();

        for attr in attrs {
            if attr.name.as_str() == "doc" {
                if let Maybe::Some(args) = &attr.args {
                    for arg in args.iter() {
                        // @doc("some text") — extract the string literal
                        if let verum_ast::expr::ExprKind::Literal(lit) = &arg.kind {
                            if let verum_ast::literal::LiteralKind::Text(s) = &lit.kind {
                                doc_parts.push(Text::from(s.as_str()));
                            }
                        }
                    }
                }
            }
        }

        if doc_parts.is_empty() {
            Maybe::None
        } else {
            Maybe::Some(doc_parts.iter().map(|t| t.as_str()).collect::<Vec<_>>().join("\n").into())
        }
    }

    fn ast_type_to_type(&self, ast_ty: &verum_ast::ty::Type) -> Type {
        use verum_ast::ty::TypeKind;

        match &ast_ty.kind {
            TypeKind::Unit => Type::Unit,
            TypeKind::Bool => Type::Bool,
            TypeKind::Int => Type::Int,
            TypeKind::Float => Type::Float,
            TypeKind::Char => Type::Char,
            TypeKind::Text => Type::Text,
            TypeKind::Never => Type::Never,

            TypeKind::Path(path) => {
                Type::Named {
                    path: path.clone(),
                    args: List::new(),
                }
            }

            TypeKind::Generic { base, args } => {
                let base_type = self.ast_type_to_type(base);
                let type_args: List<Type> = args.iter().filter_map(|arg| {
                    use verum_ast::ty::GenericArg;
                    match arg {
                        GenericArg::Type(ty) => Some(self.ast_type_to_type(ty)),
                        _ => None,
                    }
                }).collect();

                match base_type {
                    Type::Named { path, .. } => Type::Named { path, args: type_args },
                    _ => base_type,
                }
            }

            TypeKind::Reference { inner, mutable, .. } => {
                Type::Reference {
                    inner: Box::new(self.ast_type_to_type(inner)),
                    mutable: *mutable,
                }
            }

            TypeKind::Tuple(types) => {
                let tys = types.iter().map(|t| self.ast_type_to_type(t)).collect();
                Type::Tuple(tys)
            }

            TypeKind::Array { element, .. } => {
                Type::Array {
                    element: Box::new(self.ast_type_to_type(element)),
                    size: Maybe::None,
                }
            }

            TypeKind::Function { params, return_type, .. } => {
                let param_types: List<Type> = params.iter()
                    .map(|p| self.ast_type_to_type(p))
                    .collect();
                let ret_type = self.ast_type_to_type(return_type);
                Type::function(param_types, ret_type)
            }

            _ => Type::Unit, // Fallback for unhandled cases
        }
    }
}

// =============================================================================
// Binary Serialization for ModuleExports
// =============================================================================
//
// Custom binary format (tag-based, self-contained, no serde needed):
//   - Magic: b"VTYP" (4 bytes)
//   - Version: u8
//   - String: u32 length + UTF-8 bytes
//   - List<T>: u32 count + items
//   - Type: u8 tag + variant-specific data
//   - Bool: u8 (0/1)
//
// =============================================================================

const VTYP_MAGIC: &[u8; 4] = b"VTYP";
const VTYP_VERSION: u8 = 1;

// Type tags
const TAG_UNIT: u8 = 0;
const TAG_NEVER: u8 = 1;
const TAG_UNKNOWN: u8 = 2;
const TAG_BOOL: u8 = 3;
const TAG_INT: u8 = 4;
const TAG_FLOAT: u8 = 5;
const TAG_CHAR: u8 = 6;
const TAG_TEXT: u8 = 7;
const TAG_NAMED: u8 = 8;
const TAG_GENERIC: u8 = 9;
const TAG_FUNCTION: u8 = 10;
const TAG_TUPLE: u8 = 11;
const TAG_ARRAY: u8 = 12;
const TAG_REFERENCE: u8 = 13;
const TAG_CHECKED_REF: u8 = 14;
const TAG_UNSAFE_REF: u8 = 15;
const TAG_RECORD: u8 = 16;
const TAG_VARIANT: u8 = 17;
const TAG_DYN_PROTOCOL: u8 = 18;
const TAG_SLICE: u8 = 19;
const TAG_VAR: u8 = 20;
const TAG_POINTER: u8 = 21;
const TAG_FORALL: u8 = 22;
const TAG_META: u8 = 23;
const TAG_OWNERSHIP: u8 = 24;
const TAG_VOLATILE_PTR: u8 = 25;
const TAG_REFINED: u8 = 26;
const TAG_EXISTS: u8 = 27;
const TAG_EXT_RECORD: u8 = 28;
// Fallback for unhandled types
const TAG_OPAQUE: u8 = 255;

struct BinaryWriter {
    buf: List<u8>,
}

impl BinaryWriter {
    fn new() -> Self {
        Self { buf: List::new() }
    }

    fn write_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    fn write_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn write_bool(&mut self, v: bool) {
        self.buf.push(if v { 1 } else { 0 });
    }

    fn write_str(&mut self, s: &str) {
        let bytes = s.as_bytes();
        self.write_u32(bytes.len() as u32);
        self.buf.extend_from_slice(bytes);
    }

    fn write_text(&mut self, t: &Text) {
        self.write_str(t.as_str());
    }

    fn write_type(&mut self, ty: &Type) {
        match ty {
            Type::Unit => self.write_u8(TAG_UNIT),
            Type::Never => self.write_u8(TAG_NEVER),
            Type::Unknown => self.write_u8(TAG_UNKNOWN),
            Type::Bool => self.write_u8(TAG_BOOL),
            Type::Int => self.write_u8(TAG_INT),
            Type::Float => self.write_u8(TAG_FLOAT),
            Type::Char => self.write_u8(TAG_CHAR),
            Type::Text => self.write_u8(TAG_TEXT),

            Type::Var(tv) => {
                self.write_u8(TAG_VAR);
                self.write_u32(tv.id() as u32);
            }

            Type::Named { path, args } => {
                self.write_u8(TAG_NAMED);
                // Serialize path as its string representation
                let path_str = format!("{}", path);
                self.write_str(&path_str);
                self.write_u32(args.len() as u32);
                for arg in args.iter() {
                    self.write_type(arg);
                }
            }

            Type::Generic { name, args } => {
                self.write_u8(TAG_GENERIC);
                self.write_text(name);
                self.write_u32(args.len() as u32);
                for arg in args.iter() {
                    self.write_type(arg);
                }
            }

            Type::Function { params, return_type, contexts: _, type_params: _, properties: _ } => {
                self.write_u8(TAG_FUNCTION);
                self.write_u32(params.len() as u32);
                for p in params.iter() {
                    self.write_type(p);
                }
                self.write_type(return_type);
                // contexts, type_params, properties are omitted for now
                // (complex nested types — will be added in v2)
            }

            Type::Tuple(types) => {
                self.write_u8(TAG_TUPLE);
                self.write_u32(types.len() as u32);
                for t in types.iter() {
                    self.write_type(t);
                }
            }

            Type::Array { element, size } => {
                self.write_u8(TAG_ARRAY);
                self.write_type(element);
                match size {
                    Some(n) => {
                        self.write_bool(true);
                        self.write_u32(*n as u32);
                    }
                    None => self.write_bool(false),
                }
            }

            Type::Slice { element } => {
                self.write_u8(TAG_SLICE);
                self.write_type(element);
            }

            Type::Reference { mutable, inner } => {
                self.write_u8(TAG_REFERENCE);
                self.write_bool(*mutable);
                self.write_type(inner);
            }

            Type::CheckedReference { mutable, inner } => {
                self.write_u8(TAG_CHECKED_REF);
                self.write_bool(*mutable);
                self.write_type(inner);
            }

            Type::UnsafeReference { mutable, inner } => {
                self.write_u8(TAG_UNSAFE_REF);
                self.write_bool(*mutable);
                self.write_type(inner);
            }

            Type::Ownership { mutable, inner } => {
                self.write_u8(TAG_OWNERSHIP);
                self.write_bool(*mutable);
                self.write_type(inner);
            }

            Type::Pointer { mutable, inner } => {
                self.write_u8(TAG_POINTER);
                self.write_bool(*mutable);
                self.write_type(inner);
            }

            Type::VolatilePointer { mutable, inner } => {
                self.write_u8(TAG_VOLATILE_PTR);
                self.write_bool(*mutable);
                self.write_type(inner);
            }

            Type::Record(fields) => {
                self.write_u8(TAG_RECORD);
                self.write_u32(fields.len() as u32);
                for (name, ty) in fields.iter() {
                    self.write_text(name);
                    self.write_type(ty);
                }
            }

            Type::ExtensibleRecord { fields, row_var } => {
                self.write_u8(TAG_EXT_RECORD);
                self.write_u32(fields.len() as u32);
                for (name, ty) in fields.iter() {
                    self.write_text(name);
                    self.write_type(ty);
                }
                match row_var {
                    Some(tv) => {
                        self.write_bool(true);
                        self.write_u32(tv.id() as u32);
                    }
                    None => self.write_bool(false),
                }
            }

            Type::Variant(variants) => {
                self.write_u8(TAG_VARIANT);
                self.write_u32(variants.len() as u32);
                for (name, ty) in variants.iter() {
                    self.write_text(name);
                    self.write_type(ty);
                }
            }

            Type::DynProtocol { bounds, bindings } => {
                self.write_u8(TAG_DYN_PROTOCOL);
                self.write_u32(bounds.len() as u32);
                for b in bounds.iter() {
                    self.write_text(b);
                }
                self.write_u32(bindings.len() as u32);
                for (k, v) in bindings.iter() {
                    self.write_text(k);
                    self.write_type(v);
                }
            }

            Type::Forall { vars, body } => {
                self.write_u8(TAG_FORALL);
                self.write_u32(vars.len() as u32);
                for v in vars.iter() {
                    self.write_u32(v.id() as u32);
                }
                self.write_type(body);
            }

            Type::Meta { name, ty, .. } => {
                self.write_u8(TAG_META);
                self.write_text(name);
                self.write_type(ty);
            }

            Type::Refined { base, .. } => {
                // Encode as base type (predicates are not serialized — regenerated at check time)
                self.write_u8(TAG_REFINED);
                self.write_type(base);
            }

            Type::Exists { var, body } => {
                self.write_u8(TAG_EXISTS);
                self.write_u32(var.id() as u32);
                self.write_type(body);
            }

            // Fallback for any variant not covered
            _ => {
                self.write_u8(TAG_OPAQUE);
                let repr = format!("{:?}", ty);
                self.write_str(&repr);
            }
        }
    }

    fn write_type_param(&mut self, tp: &TypeParam) {
        self.write_text(&tp.name);
        self.write_u32(tp.bounds.len() as u32);
        for b in tp.bounds.iter() {
            self.write_text(b);
        }
        match &tp.default {
            Maybe::Some(ty) => {
                self.write_bool(true);
                self.write_type(ty);
            }
            Maybe::None => self.write_bool(false),
        }
    }

    fn write_field(&mut self, f: &ExportedField) {
        self.write_text(&f.name);
        self.write_type(&f.ty);
        self.write_bool(f.is_public);
    }

    fn write_variant_payload(&mut self, vp: &VariantPayload) {
        match vp {
            VariantPayload::Unit => self.write_u8(0),
            VariantPayload::Tuple(types) => {
                self.write_u8(1);
                self.write_u32(types.len() as u32);
                for t in types.iter() {
                    self.write_type(t);
                }
            }
            VariantPayload::Struct(fields) => {
                self.write_u8(2);
                self.write_u32(fields.len() as u32);
                for f in fields.iter() {
                    self.write_field(f);
                }
            }
        }
    }

    fn write_method(&mut self, m: &ExportedMethod) {
        self.write_text(&m.name);
        self.write_u32(m.type_params.len() as u32);
        for tp in m.type_params.iter() {
            self.write_type_param(tp);
        }
        self.write_u32(m.params.len() as u32);
        for p in m.params.iter() {
            self.write_text(&p.name);
            self.write_type(&p.ty);
        }
        self.write_type(&m.return_type);
        self.write_bool(m.is_static);
        self.write_bool(m.is_async);
    }

    fn write_exports(&mut self, exports: &ModuleExports) {
        // Magic + version
        self.buf.extend_from_slice(VTYP_MAGIC);
        self.write_u8(VTYP_VERSION);

        // Module path
        self.write_text(&exports.path);

        // Types
        self.write_u32(exports.types.len() as u32);
        for et in exports.types.iter() {
            self.write_text(&et.name);
            self.write_u32(et.type_params.len() as u32);
            for tp in et.type_params.iter() {
                self.write_type_param(tp);
            }
            // TypeDefinition tag + data
            match &et.definition {
                TypeDefinition::Record { fields } => {
                    self.write_u8(0);
                    self.write_u32(fields.len() as u32);
                    for f in fields.iter() {
                        self.write_field(f);
                    }
                }
                TypeDefinition::Variant { variants } => {
                    self.write_u8(1);
                    self.write_u32(variants.len() as u32);
                    for v in variants.iter() {
                        self.write_text(&v.name);
                        self.write_variant_payload(&v.payload);
                    }
                }
                TypeDefinition::Alias { target } => {
                    self.write_u8(2);
                    self.write_type(target);
                }
                TypeDefinition::Newtype { inner } => {
                    self.write_u8(3);
                    self.write_type(inner);
                }
                TypeDefinition::Protocol { methods, associated_types } => {
                    self.write_u8(4);
                    self.write_u32(methods.len() as u32);
                    for m in methods.iter() {
                        self.write_method(m);
                    }
                    self.write_u32(associated_types.len() as u32);
                    for at in associated_types.iter() {
                        self.write_text(&at.name);
                        self.write_u32(at.bounds.len() as u32);
                        for b in at.bounds.iter() {
                            self.write_text(b);
                        }
                        match &at.default {
                            Maybe::Some(ty) => {
                                self.write_bool(true);
                                self.write_type(ty);
                            }
                            Maybe::None => self.write_bool(false),
                        }
                    }
                }
            }
            self.write_bool(et.is_public);
            match &et.doc {
                Maybe::Some(d) => {
                    self.write_bool(true);
                    self.write_text(d);
                }
                Maybe::None => self.write_bool(false),
            }
        }

        // Impl blocks
        self.write_u32(exports.impls.len() as u32);
        for imp in exports.impls.iter() {
            self.write_type(&imp.for_type);
            match &imp.protocol {
                Maybe::Some(p) => {
                    self.write_bool(true);
                    self.write_text(p);
                }
                Maybe::None => self.write_bool(false),
            }
            self.write_u32(imp.type_params.len() as u32);
            for tp in imp.type_params.iter() {
                self.write_type_param(tp);
            }
            self.write_u32(imp.methods.len() as u32);
            for m in imp.methods.iter() {
                self.write_method(m);
            }
        }

        // Re-exports
        self.write_u32(exports.re_exports.len() as u32);
        for re in exports.re_exports.iter() {
            self.write_text(&re.local_name);
            self.write_text(&re.source_module);
            self.write_text(&re.source_name);
        }

        // Module-level functions
        self.write_u32(exports.functions.len() as u32);
        for f in exports.functions.iter() {
            self.write_method(f);
        }
    }
}

struct BinaryReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BinaryReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read_u8(&mut self) -> Option<u8> {
        if self.pos < self.data.len() {
            let v = self.data[self.pos];
            self.pos += 1;
            Some(v)
        } else {
            None
        }
    }

    fn read_u32(&mut self) -> Option<u32> {
        if self.pos + 4 <= self.data.len() {
            let v = u32::from_le_bytes([
                self.data[self.pos],
                self.data[self.pos + 1],
                self.data[self.pos + 2],
                self.data[self.pos + 3],
            ]);
            self.pos += 4;
            Some(v)
        } else {
            None
        }
    }

    fn read_bool(&mut self) -> Option<bool> {
        self.read_u8().map(|v| v != 0)
    }

    fn read_text(&mut self) -> Option<Text> {
        let len = self.read_u32()? as usize;
        if self.pos + len <= self.data.len() {
            let s = std::str::from_utf8(&self.data[self.pos..self.pos + len]).ok()?;
            self.pos += len;
            Some(Text::from(s))
        } else {
            None
        }
    }

    fn read_type(&mut self) -> Option<Type> {
        let tag = self.read_u8()?;
        match tag {
            TAG_UNIT => Some(Type::Unit),
            TAG_NEVER => Some(Type::Never),
            TAG_UNKNOWN => Some(Type::Unknown),
            TAG_BOOL => Some(Type::Bool),
            TAG_INT => Some(Type::Int),
            TAG_FLOAT => Some(Type::Float),
            TAG_CHAR => Some(Type::Char),
            TAG_TEXT => Some(Type::Text),

            TAG_VAR => {
                let id = self.read_u32()?;
                Some(Type::Var(TypeVar::new(id as usize)))
            }

            TAG_NAMED => {
                let path_str = self.read_text()?;
                let count = self.read_u32()? as usize;
                let mut args = List::new();
                for _ in 0..count {
                    args.push(self.read_type()?);
                }
                // Reconstruct Path from string
                let path = verum_ast::ty::Path::single(verum_ast::Ident::new(path_str, verum_ast::span::Span::dummy()));
                Some(Type::Named { path, args })
            }

            TAG_GENERIC => {
                let name = self.read_text()?;
                let count = self.read_u32()? as usize;
                let mut args = List::new();
                for _ in 0..count {
                    args.push(self.read_type()?);
                }
                Some(Type::Generic { name, args })
            }

            TAG_FUNCTION => {
                let param_count = self.read_u32()? as usize;
                let mut params = List::new();
                for _ in 0..param_count {
                    params.push(self.read_type()?);
                }
                let return_type = self.read_type()?;
                Some(Type::Function {
                    params,
                    return_type: Box::new(return_type),
                    contexts: None,
                    type_params: List::new(),
                    properties: None,
                })
            }

            TAG_TUPLE => {
                let count = self.read_u32()? as usize;
                let mut types = List::new();
                for _ in 0..count {
                    types.push(self.read_type()?);
                }
                Some(Type::Tuple(types))
            }

            TAG_ARRAY => {
                let element = self.read_type()?;
                let has_size = self.read_bool()?;
                let size = if has_size {
                    Some(self.read_u32()? as usize)
                } else {
                    None
                };
                Some(Type::Array { element: Box::new(element), size })
            }

            TAG_SLICE => {
                let element = self.read_type()?;
                Some(Type::Slice { element: Box::new(element) })
            }

            TAG_REFERENCE => {
                let mutable = self.read_bool()?;
                let inner = self.read_type()?;
                Some(Type::Reference { mutable, inner: Box::new(inner) })
            }

            TAG_CHECKED_REF => {
                let mutable = self.read_bool()?;
                let inner = self.read_type()?;
                Some(Type::CheckedReference { mutable, inner: Box::new(inner) })
            }

            TAG_UNSAFE_REF => {
                let mutable = self.read_bool()?;
                let inner = self.read_type()?;
                Some(Type::UnsafeReference { mutable, inner: Box::new(inner) })
            }

            TAG_OWNERSHIP => {
                let mutable = self.read_bool()?;
                let inner = self.read_type()?;
                Some(Type::Ownership { mutable, inner: Box::new(inner) })
            }

            TAG_POINTER => {
                let mutable = self.read_bool()?;
                let inner = self.read_type()?;
                Some(Type::Pointer { mutable, inner: Box::new(inner) })
            }

            TAG_VOLATILE_PTR => {
                let mutable = self.read_bool()?;
                let inner = self.read_type()?;
                Some(Type::VolatilePointer { mutable, inner: Box::new(inner) })
            }

            TAG_RECORD => {
                let count = self.read_u32()? as usize;
                let mut fields = indexmap::IndexMap::new();
                for _ in 0..count {
                    let name = self.read_text()?;
                    let ty = self.read_type()?;
                    fields.insert(name, ty);
                }
                Some(Type::Record(fields))
            }

            TAG_EXT_RECORD => {
                let count = self.read_u32()? as usize;
                let mut fields = indexmap::IndexMap::new();
                for _ in 0..count {
                    let name = self.read_text()?;
                    let ty = self.read_type()?;
                    fields.insert(name, ty);
                }
                let has_row = self.read_bool()?;
                let row_var = if has_row {
                    Some(TypeVar::new(self.read_u32()? as usize))
                } else {
                    None
                };
                Some(Type::ExtensibleRecord { fields, row_var })
            }

            TAG_VARIANT => {
                let count = self.read_u32()? as usize;
                let mut variants = indexmap::IndexMap::new();
                for _ in 0..count {
                    let name = self.read_text()?;
                    let ty = self.read_type()?;
                    variants.insert(name, ty);
                }
                Some(Type::Variant(variants))
            }

            TAG_DYN_PROTOCOL => {
                let bounds_count = self.read_u32()? as usize;
                let mut bounds = List::new();
                for _ in 0..bounds_count {
                    bounds.push(self.read_text()?);
                }
                let bindings_count = self.read_u32()? as usize;
                let mut bindings = Map::new();
                for _ in 0..bindings_count {
                    let k = self.read_text()?;
                    let v = self.read_type()?;
                    bindings.insert(k, v);
                }
                Some(Type::DynProtocol { bounds, bindings })
            }

            TAG_FORALL => {
                let var_count = self.read_u32()? as usize;
                let mut vars = List::new();
                for _ in 0..var_count {
                    vars.push(TypeVar::new(self.read_u32()? as usize));
                }
                let body = self.read_type()?;
                Some(Type::Forall { vars, body: Box::new(body) })
            }

            TAG_META => {
                let name = self.read_text()?;
                let ty = self.read_type()?;
                Some(Type::Meta {
                    name,
                    ty: Box::new(ty),
                    refinement: None,
                    value: None,
                })
            }

            TAG_REFINED => {
                let base = self.read_type()?;
                // Return base type — refinement predicates are regenerated
                Some(*Box::new(base))
            }

            TAG_EXISTS => {
                let var = TypeVar::new(self.read_u32()? as usize);
                let body = self.read_type()?;
                Some(Type::Exists { var, body: Box::new(body) })
            }

            TAG_OPAQUE => {
                // Read debug representation but return Unknown
                let _repr = self.read_text()?;
                Some(Type::Unknown)
            }

            _ => None,
        }
    }

    fn read_type_param(&mut self) -> Option<TypeParam> {
        let name = self.read_text()?;
        let bounds_count = self.read_u32()? as usize;
        let mut bounds = List::new();
        for _ in 0..bounds_count {
            bounds.push(self.read_text()?);
        }
        let has_default = self.read_bool()?;
        let default = if has_default {
            Maybe::Some(self.read_type()?)
        } else {
            Maybe::None
        };
        Some(TypeParam { name, bounds, default })
    }

    fn read_field(&mut self) -> Option<ExportedField> {
        let name = self.read_text()?;
        let ty = self.read_type()?;
        let is_public = self.read_bool()?;
        Some(ExportedField { name, ty, is_public })
    }

    fn read_variant_payload(&mut self) -> Option<VariantPayload> {
        let tag = self.read_u8()?;
        match tag {
            0 => Some(VariantPayload::Unit),
            1 => {
                let count = self.read_u32()? as usize;
                let mut types = List::new();
                for _ in 0..count {
                    types.push(self.read_type()?);
                }
                Some(VariantPayload::Tuple(types))
            }
            2 => {
                let count = self.read_u32()? as usize;
                let mut fields = List::new();
                for _ in 0..count {
                    fields.push(self.read_field()?);
                }
                Some(VariantPayload::Struct(fields))
            }
            _ => None,
        }
    }

    fn read_method(&mut self) -> Option<ExportedMethod> {
        let name = self.read_text()?;
        let tp_count = self.read_u32()? as usize;
        let mut type_params = List::new();
        for _ in 0..tp_count {
            type_params.push(self.read_type_param()?);
        }
        let param_count = self.read_u32()? as usize;
        let mut params = List::new();
        for _ in 0..param_count {
            let pname = self.read_text()?;
            let pty = self.read_type()?;
            params.push(MethodParam { name: pname, ty: pty });
        }
        let return_type = self.read_type()?;
        let is_static = self.read_bool()?;
        let is_async = self.read_bool()?;
        Some(ExportedMethod { name, type_params, params, return_type, is_static, is_async })
    }

    fn read_exports(&mut self) -> Option<ModuleExports> {
        // Check magic
        if self.remaining() < 5 { return None; }
        if &self.data[self.pos..self.pos + 4] != VTYP_MAGIC { return None; }
        self.pos += 4;
        let version = self.read_u8()?;
        if version != VTYP_VERSION { return None; }

        let path = self.read_text()?;

        // Types
        let type_count = self.read_u32()? as usize;
        let mut types = List::new();
        for _ in 0..type_count {
            let name = self.read_text()?;
            let tp_count = self.read_u32()? as usize;
            let mut type_params = List::new();
            for _ in 0..tp_count {
                type_params.push(self.read_type_param()?);
            }
            let def_tag = self.read_u8()?;
            let definition = match def_tag {
                0 => {
                    let fcount = self.read_u32()? as usize;
                    let mut fields = List::new();
                    for _ in 0..fcount {
                        fields.push(self.read_field()?);
                    }
                    TypeDefinition::Record { fields }
                }
                1 => {
                    let vcount = self.read_u32()? as usize;
                    let mut variants = List::new();
                    for _ in 0..vcount {
                        let vname = self.read_text()?;
                        let payload = self.read_variant_payload()?;
                        variants.push(ExportedVariant { name: vname, payload });
                    }
                    TypeDefinition::Variant { variants }
                }
                2 => {
                    let target = self.read_type()?;
                    TypeDefinition::Alias { target }
                }
                3 => {
                    let inner = self.read_type()?;
                    TypeDefinition::Newtype { inner }
                }
                4 => {
                    let mcount = self.read_u32()? as usize;
                    let mut methods = List::new();
                    for _ in 0..mcount {
                        methods.push(self.read_method()?);
                    }
                    let at_count = self.read_u32()? as usize;
                    let mut associated_types = List::new();
                    for _ in 0..at_count {
                        let atname = self.read_text()?;
                        let bcount = self.read_u32()? as usize;
                        let mut atbounds = List::new();
                        for _ in 0..bcount {
                            atbounds.push(self.read_text()?);
                        }
                        let has_default = self.read_bool()?;
                        let atdefault = if has_default {
                            Maybe::Some(self.read_type()?)
                        } else {
                            Maybe::None
                        };
                        associated_types.push(AssociatedType {
                            name: atname,
                            bounds: atbounds,
                            default: atdefault,
                        });
                    }
                    TypeDefinition::Protocol { methods, associated_types }
                }
                _ => return None,
            };
            let is_public = self.read_bool()?;
            let has_doc = self.read_bool()?;
            let doc = if has_doc { Maybe::Some(self.read_text()?) } else { Maybe::None };
            types.push(ExportedType { name, type_params, definition, is_public, doc });
        }

        // Impls
        let impl_count = self.read_u32()? as usize;
        let mut impls = List::new();
        for _ in 0..impl_count {
            let for_type = self.read_type()?;
            let has_protocol = self.read_bool()?;
            let protocol = if has_protocol { Maybe::Some(self.read_text()?) } else { Maybe::None };
            let tp_count = self.read_u32()? as usize;
            let mut type_params = List::new();
            for _ in 0..tp_count {
                type_params.push(self.read_type_param()?);
            }
            let mcount = self.read_u32()? as usize;
            let mut methods = List::new();
            for _ in 0..mcount {
                methods.push(self.read_method()?);
            }
            impls.push(ExportedImpl { for_type, protocol, type_params, methods });
        }

        // Re-exports
        let re_count = self.read_u32()? as usize;
        let mut re_exports = List::new();
        for _ in 0..re_count {
            let local_name = self.read_text()?;
            let source_module = self.read_text()?;
            let source_name = self.read_text()?;
            re_exports.push(ReExport { local_name, source_module, source_name });
        }

        // Functions
        let fn_count = self.read_u32()? as usize;
        let mut functions = List::new();
        for _ in 0..fn_count {
            functions.push(self.read_method()?);
        }

        Some(ModuleExports { path, types, impls, re_exports, functions })
    }
}

/// Serialize ModuleExports to binary format for VBC
pub fn serialize_module_exports(exports: &ModuleExports) -> List<u8> {
    let mut writer = BinaryWriter::new();
    writer.write_exports(exports);
    writer.buf
}

/// Deserialize ModuleExports from binary format
pub fn deserialize_module_exports(data: &[u8]) -> Maybe<ModuleExports> {
    let mut reader = BinaryReader::new(data);
    reader.read_exports()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_exporter_basic() {
        let methods: Map<Text, Map<Text, TypeScheme>> = Map::new();
        let exporter = TypeExporter::new(&methods);
        assert!(exporter.exports.types.is_empty());
    }

    #[test]
    fn test_serialize_empty_module() {
        let exports = ModuleExports {
            path: Text::from("test/empty"),
            types: List::new(),
            impls: List::new(),
            re_exports: List::new(),
            functions: List::new(),
        };
        let data = serialize_module_exports(&exports);
        assert!(!data.is_empty());
        let deserialized = deserialize_module_exports(&data);
        match deserialized {
            Maybe::Some(result) => {
                assert_eq!(result.path.as_str(), "test/empty");
                assert!(result.types.is_empty());
                assert!(result.impls.is_empty());
            }
            Maybe::None => panic!("deserialization failed"),
        }
    }

    #[test]
    fn test_serialize_record_type() {
        let exports = ModuleExports {
            path: Text::from("test/point"),
            types: vec![ExportedType {
                name: Text::from("Point"),
                type_params: List::new(),
                definition: TypeDefinition::Record {
                    fields: vec![
                        ExportedField { name: Text::from("x"), ty: Type::Float, is_public: true },
                        ExportedField { name: Text::from("y"), ty: Type::Float, is_public: true },
                    ].into(),
                },
                is_public: true,
                doc: Maybe::Some(Text::from("A 2D point")),
            }].into(),
            impls: List::new(),
            re_exports: List::new(),
            functions: List::new(),
        };
        let data = serialize_module_exports(&exports);
        let deserialized = deserialize_module_exports(&data);
        match deserialized {
            Maybe::Some(result) => {
                assert_eq!(result.types.len(), 1);
                assert_eq!(result.types[0].name.as_str(), "Point");
                assert!(result.types[0].is_public);
                match &result.types[0].doc {
                    Maybe::Some(d) => assert_eq!(d.as_str(), "A 2D point"),
                    Maybe::None => panic!("expected doc"),
                }
                if let TypeDefinition::Record { fields } = &result.types[0].definition {
                    assert_eq!(fields.len(), 2);
                    assert_eq!(fields[0].name.as_str(), "x");
                    assert_eq!(fields[0].ty, Type::Float);
                } else {
                    panic!("expected Record definition");
                }
            }
            Maybe::None => panic!("deserialization failed"),
        }
    }

    #[test]
    fn test_serialize_variant_type() {
        let exports = ModuleExports {
            path: Text::from("test/option"),
            types: vec![ExportedType {
                name: Text::from("Maybe"),
                type_params: vec![TypeParam {
                    name: Text::from("T"),
                    bounds: List::new(),
                    default: Maybe::None,
                }].into(),
                definition: TypeDefinition::Variant {
                    variants: vec![
                        ExportedVariant { name: Text::from("None"), payload: VariantPayload::Unit },
                        ExportedVariant {
                            name: Text::from("Some"),
                            payload: VariantPayload::Tuple(vec![Type::Var(TypeVar::new(0))].into()),
                        },
                    ].into(),
                },
                is_public: true,
                doc: Maybe::None,
            }].into(),
            impls: List::new(),
            re_exports: List::new(),
            functions: List::new(),
        };
        let data = serialize_module_exports(&exports);
        let deserialized = deserialize_module_exports(&data);
        match deserialized {
            Maybe::Some(result) => {
                assert_eq!(result.types[0].name.as_str(), "Maybe");
                assert_eq!(result.types[0].type_params.len(), 1);
                assert_eq!(result.types[0].type_params[0].name.as_str(), "T");
                if let TypeDefinition::Variant { variants } = &result.types[0].definition {
                    assert_eq!(variants.len(), 2);
                    assert_eq!(variants[0].name.as_str(), "None");
                    assert!(matches!(variants[0].payload, VariantPayload::Unit));
                    assert_eq!(variants[1].name.as_str(), "Some");
                } else {
                    panic!("expected Variant definition");
                }
            }
            Maybe::None => panic!("deserialization failed"),
        }
    }

    #[test]
    fn test_serialize_function() {
        let exports = ModuleExports {
            path: Text::from("test/math"),
            types: List::new(),
            impls: List::new(),
            re_exports: List::new(),
            functions: vec![ExportedMethod {
                name: Text::from("add"),
                type_params: List::new(),
                params: vec![
                    MethodParam { name: Text::from("a"), ty: Type::Int },
                    MethodParam { name: Text::from("b"), ty: Type::Int },
                ].into(),
                return_type: Type::Int,
                is_static: true,
                is_async: false,
            }].into(),
        };
        let data = serialize_module_exports(&exports);
        let deserialized = deserialize_module_exports(&data);
        match deserialized {
            Maybe::Some(result) => {
                assert_eq!(result.functions.len(), 1);
                assert_eq!(result.functions[0].name.as_str(), "add");
                assert_eq!(result.functions[0].params.len(), 2);
                assert_eq!(result.functions[0].return_type, Type::Int);
            }
            Maybe::None => panic!("deserialization failed"),
        }
    }

    #[test]
    fn test_serialize_impl_block() {
        let exports = ModuleExports {
            path: Text::from("test/display"),
            types: List::new(),
            impls: vec![ExportedImpl {
                for_type: Type::Named {
                    path: verum_ast::ty::Path::single(verum_ast::Ident::new("Point", verum_ast::span::Span::dummy())),
                    args: List::new(),
                },
                protocol: Maybe::Some(Text::from("Display")),
                type_params: List::new(),
                methods: vec![ExportedMethod {
                    name: Text::from("fmt"),
                    type_params: List::new(),
                    params: List::new(),
                    return_type: Type::Unit,
                    is_static: false,
                    is_async: false,
                }].into(),
            }].into(),
            re_exports: List::new(),
            functions: List::new(),
        };
        let data = serialize_module_exports(&exports);
        let deserialized = deserialize_module_exports(&data);
        match deserialized {
            Maybe::Some(result) => {
                assert_eq!(result.impls.len(), 1);
                match &result.impls[0].protocol {
                    Maybe::Some(p) => assert_eq!(p.as_str(), "Display"),
                    Maybe::None => panic!("expected protocol"),
                }
                assert_eq!(result.impls[0].methods.len(), 1);
            }
            Maybe::None => panic!("deserialization failed"),
        }
    }
}
