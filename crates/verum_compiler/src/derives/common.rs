//! Common types and utilities for derive macros
//!
//! This module provides shared infrastructure for all derive implementations:
//! - TypeInfo: Extracted information about the type being derived
//! - FieldInfo: Information about struct/record fields
//! - VariantInfo: Information about enum/variant cases
//! - DeriveContext: Complete context for derive expansion
//! - DeriveError: Error types for derive failures

use verum_ast::Span;
use verum_ast::decl::{
    FunctionBody, FunctionDecl, FunctionParam, FunctionParamKind, ImplDecl, ImplItem, ImplItemKind,
    ImplKind, Item, ItemKind, RecordField, TypeDecl, TypeDeclBody, Variant, VariantData,
    Visibility,
};
use verum_ast::expr::{Block, Expr, ExprKind};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::ty::{GenericParam, GenericParamKind, Ident, Path, PathSegment, Type, TypeKind};
use verum_common::{List, Text};

/// Helper to create a Path from a dot-separated string like "Foo.Bar"
pub fn path_from_str(s: &str, span: Span) -> Path {
    let segments: Vec<PathSegment> = s
        .split('.')
        .map(|part| PathSegment::Name(Ident::new(part, span)))
        .collect();
    Path::new(segments.into(), span)
}

/// Information about a field in a struct/record
///
/// # Builder Pattern Support
///
/// Fields with `has_default = true` are optional in @builder:
/// - Required fields (no default): `Maybe<T>` in builder type
/// - Optional fields (has default): `T` in builder type, uses default if not set
///
/// Builder pattern derive: generates .with_field() methods for record types.
#[derive(Debug, Clone)]
pub struct FieldInfo {
    /// Field name
    pub name: Text,
    /// Field type
    pub ty: Type,
    /// Field index (for positional access)
    pub index: usize,
    /// Whether the field is public
    pub is_public: bool,
    /// Whether the field has a default value (optional in @builder)
    pub has_default: bool,
    /// The default value expression (if any)
    pub default_value: verum_common::Maybe<Expr>,
    /// Field-level attributes — surfaced for derives that key off them
    /// (e.g. @flag / @positional in @derive(ShellRender)).
    pub attributes: List<verum_ast::Attribute>,
    /// Source span
    pub span: Span,
}

impl FieldInfo {
    /// Create from a record field definition
    pub fn from_record_field(field: &RecordField, index: usize) -> Self {
        Self {
            name: Text::from(field.name.as_str()),
            ty: field.ty.clone(),
            index,
            is_public: field.visibility == Visibility::Public,
            has_default: field.has_default(),
            default_value: field.default_value.clone(),
            attributes: field.attributes.clone(),
            span: field.span,
        }
    }

    /// True iff the field's declared type is `Bool`.
    pub fn is_bool(&self) -> bool {
        match &self.ty.kind {
            verum_ast::ty::TypeKind::Path(path) =>
                path.last_segment_name() == "Bool",
            _ => false,
        }
    }

    /// True iff the declared type is `List<...>`.
    pub fn is_list(&self) -> bool {
        match &self.ty.kind {
            verum_ast::ty::TypeKind::Path(path) =>
                path.last_segment_name() == "List",
            _ => false,
        }
    }

    /// True iff the declared type is `Maybe<...>`.
    pub fn is_maybe(&self) -> bool {
        match &self.ty.kind {
            verum_ast::ty::TypeKind::Path(path) =>
                path.last_segment_name() == "Maybe",
            _ => false,
        }
    }

    /// Check if this field is required (no default value for @builder)
    pub fn is_required(&self) -> bool {
        !self.has_default
    }

    /// Get the field as an identifier
    pub fn ident(&self, span: Span) -> Ident {
        Ident::new(self.name.as_str(), span)
    }

    /// Create a field access expression on self
    pub fn access_on_self(&self, span: Span) -> Expr {
        let self_expr = Expr::new(ExprKind::Path(Path::single(Ident::new("self", span))), span);
        Expr::new(
            ExprKind::Field {
                expr: Box::new(self_expr),
                field: self.ident(span),
            },
            span,
        )
    }

    /// Create a field access expression on other
    pub fn access_on_other(&self, span: Span) -> Expr {
        let other_expr = Expr::new(
            ExprKind::Path(Path::single(Ident::new("other", span))),
            span,
        );
        Expr::new(
            ExprKind::Field {
                expr: Box::new(other_expr),
                field: self.ident(span),
            },
            span,
        )
    }
}

/// Information about a variant in an enum
#[derive(Debug, Clone)]
pub struct VariantInfo {
    /// Variant name
    pub name: Text,
    /// Variant fields (empty for unit variants)
    pub fields: List<FieldInfo>,
    /// Whether this is a unit variant
    pub is_unit: bool,
    /// Whether this is a tuple variant
    pub is_tuple: bool,
    /// Variant index
    pub index: usize,
    /// Source span
    pub span: Span,
}

impl VariantInfo {
    /// Create from a variant definition
    pub fn from_variant(variant: &Variant, index: usize) -> Self {
        let (fields, is_tuple) = match &variant.data {
            None => (List::new(), false),
            Some(VariantData::Tuple(types)) => {
                let field_infos: Vec<FieldInfo> = types
                    .iter()
                    .enumerate()
                    .map(|(i, ty)| FieldInfo {
                        name: Text::from(format!("{}", i)),
                        ty: ty.clone(),
                        index: i,
                        is_public: false,
                        has_default: false,
                        default_value: verum_common::Maybe::None,
                        // Synthesized positional fields carry no
                        // user-written attributes — those only
                        // apply to record fields with explicit
                        // names. Pin the empty default so the
                        // FieldInfo invariant is satisfied.
                        attributes: List::new(),
                        span: variant.span,
                    })
                    .collect();
                (List::from(field_infos), true)
            }
            Some(VariantData::Record(fields)) => {
                let field_infos: Vec<FieldInfo> = fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| FieldInfo::from_record_field(f, i))
                    .collect();
                (List::from(field_infos), false)
            }
        };

        let is_unit = fields.is_empty();

        Self {
            name: Text::from(variant.name.as_str()),
            fields,
            is_unit,
            is_tuple,
            index,
            span: variant.span,
        }
    }

    /// Get the variant as an identifier
    pub fn ident(&self, span: Span) -> Ident {
        Ident::new(self.name.as_str(), span)
    }
}

/// Complete type information for derive expansion
#[derive(Debug, Clone)]
pub struct TypeInfo {
    /// Type name
    pub name: Text,
    /// Generic parameters
    pub generics: List<GenericParam>,
    /// Fields (for struct/record types)
    pub fields: List<FieldInfo>,
    /// Variants (for enum types)
    pub variants: List<VariantInfo>,
    /// Whether this is an enum
    pub is_enum: bool,
    /// Whether this is a newtype (single-field wrapper)
    pub is_newtype: bool,
    /// Whether this is a refinement type
    pub is_refinement: bool,
    /// Source span
    pub span: Span,
}

impl TypeInfo {
    /// Extract type info from a TypeDecl
    pub fn from_type_decl(decl: &TypeDecl) -> Result<Self, DeriveError> {
        let name = Text::from(decl.name.as_str());
        let generics = List::from(decl.generics.clone());
        let span = decl.span;

        match &decl.body {
            TypeDeclBody::Record(fields) => {
                let field_infos: Vec<FieldInfo> = fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| FieldInfo::from_record_field(f, i))
                    .collect();

                Ok(Self {
                    name,
                    generics,
                    fields: List::from(field_infos),
                    variants: List::new(),
                    is_enum: false,
                    is_newtype: false,
                    is_refinement: false,
                    span,
                })
            }
            TypeDeclBody::Variant(variants) => {
                let variant_infos: Vec<VariantInfo> = variants
                    .iter()
                    .enumerate()
                    .map(|(i, v)| VariantInfo::from_variant(v, i))
                    .collect();

                Ok(Self {
                    name,
                    generics,
                    fields: List::new(),
                    variants: List::from(variant_infos),
                    is_enum: true,
                    is_newtype: false,
                    is_refinement: false,
                    span,
                })
            }
            TypeDeclBody::Newtype(inner_type) => {
                // Newtype is a single-field struct
                let field = FieldInfo {
                    name: Text::from("0"),
                    ty: inner_type.clone(),
                    index: 0,
                    is_public: false,
                    has_default: false,
                    default_value: verum_common::Maybe::None,
                    // Newtype's synthesized field has no attributes
                    // — the type-level @derive applies to the
                    // wrapper, not the wrapped field.
                    attributes: List::new(),
                    span,
                };

                Ok(Self {
                    name,
                    generics,
                    fields: List::from(vec![field]),
                    variants: List::new(),
                    is_enum: false,
                    is_newtype: true,
                    is_refinement: false,
                    span,
                })
            }
            TypeDeclBody::Alias(_) => Err(DeriveError::UnsupportedTypeKind {
                kind: Text::from("type alias"),
                hint: Text::from(
                    "Cannot derive for type aliases. Use the underlying type instead.",
                ),
                span,
            }),
            TypeDeclBody::Protocol(_) => Err(DeriveError::UnsupportedTypeKind {
                kind: Text::from("protocol"),
                hint: Text::from(
                    "Cannot derive for protocols. Protocols define interfaces, not data.",
                ),
                span,
            }),
            TypeDeclBody::Tuple(types) => {
                // Tuple types are similar to record types with numeric field names
                let field_infos: Vec<FieldInfo> = types
                    .iter()
                    .enumerate()
                    .map(|(i, ty)| FieldInfo {
                        name: Text::from(format!("{}", i)),
                        ty: ty.clone(),
                        index: i,
                        is_public: false,
                        has_default: false,
                        default_value: verum_common::Maybe::None,
                        // Tuple types have positional fields with
                        // no attribute syntax surface.
                        attributes: List::new(),
                        span,
                    })
                    .collect();

                Ok(Self {
                    name,
                    generics,
                    fields: List::from(field_infos),
                    variants: List::new(),
                    is_enum: false,
                    is_newtype: false,
                    is_refinement: false,
                    span,
                })
            }
            TypeDeclBody::Unit => Ok(Self {
                name,
                generics,
                fields: List::new(),
                variants: List::new(),
                is_enum: false,
                is_newtype: false,
                is_refinement: false,
                span,
            }),
            TypeDeclBody::SigmaTuple(types) => {
                // Sigma tuple types are similar to regular tuples with numeric field names
                let field_infos: Vec<FieldInfo> = types
                    .iter()
                    .enumerate()
                    .map(|(i, ty)| FieldInfo {
                        name: Text::from(format!("{}", i)),
                        ty: ty.clone(),
                        index: i,
                        is_public: false,
                        has_default: false,
                        default_value: verum_common::Maybe::None,
                        // Same as Tuple — no attribute syntax surface.
                        attributes: List::new(),
                        span,
                    })
                    .collect();

                Ok(Self {
                    name,
                    generics,
                    fields: List::from(field_infos),
                    variants: List::new(),
                    is_enum: false,
                    is_newtype: false,
                    is_refinement: false,
                    span,
                })
            }
            // Inductive/Coinductive types treated like sum types for derive purposes
            TypeDeclBody::Inductive(variants) => {
                let variant_infos: Vec<VariantInfo> = variants
                    .iter()
                    .enumerate()
                    .map(|(i, v)| VariantInfo::from_variant(v, i))
                    .collect();

                Ok(Self {
                    name,
                    generics,
                    fields: List::new(),
                    variants: List::from(variant_infos),
                    is_enum: true,
                    is_newtype: false,
                    is_refinement: false,
                    span,
                })
            }
            TypeDeclBody::Coinductive(_) => Err(DeriveError::UnsupportedTypeKind {
                kind: Text::from("coinductive"),
                hint: Text::from(
                    "Cannot derive for coinductive types. They are defined by observations, not constructors.",
                ),
                span,
            }),
            TypeDeclBody::Quotient { .. } => Err(DeriveError::UnsupportedTypeKind {
                kind: Text::from("quotient"),
                hint: Text::from(
                    "Cannot derive for quotient types directly. Derive on the underlying carrier instead, then map through `.rep`.",
                ),
                span,
            }),
        }
    }

    /// Get the type as a Path
    pub fn as_path(&self, span: Span) -> Path {
        Path::single(Ident::new(self.name.as_str(), span))
    }

    /// Get the type as a Type node
    pub fn as_type(&self, span: Span) -> Type {
        if self.generics.is_empty() {
            Type::new(TypeKind::Path(self.as_path(span)), span)
        } else {
            // Build generic type: Name<T1, T2, ...>
            let base = Type::new(TypeKind::Path(self.as_path(span)), span);
            let args: Vec<verum_ast::ty::GenericArg> = self
                .generics
                .iter()
                .filter_map(|g| match &g.kind {
                    GenericParamKind::Type { name, .. } => Some(verum_ast::ty::GenericArg::Type(
                        Type::new(TypeKind::Path(Path::single(name.clone())), span),
                    )),
                    GenericParamKind::Meta { name, .. } => Some(verum_ast::ty::GenericArg::Type(
                        Type::new(TypeKind::Path(Path::single(name.clone())), span),
                    )),
                    _ => None,
                })
                .collect();
            Type::new(
                TypeKind::Generic {
                    base: Box::new(base),
                    args: args.into(),
                },
                span,
            )
        }
    }
}

/// Context for derive macro expansion
#[derive(Debug, Clone)]
pub struct DeriveContext {
    /// Type information
    pub type_info: TypeInfo,
    /// The original type declaration
    pub type_decl: TypeDecl,
    /// Source span for errors
    pub span: Span,
}

impl DeriveContext {
    /// Create context from a TypeDecl
    pub fn from_type_decl(decl: &TypeDecl, span: Span) -> Result<Self, DeriveError> {
        let type_info = TypeInfo::from_type_decl(decl)?;
        Ok(Self {
            type_info,
            type_decl: decl.clone(),
            span,
        })
    }

    /// Generate an impl block for a protocol
    pub fn generate_impl(&self, protocol: &str, methods: List<FunctionDecl>, span: Span) -> Item {
        let items: Vec<ImplItem> = methods
            .iter()
            .map(|m| ImplItem {
                attributes: List::new(),
                visibility: Visibility::Public,
                kind: ImplItemKind::Function(m.clone()),
                span,
            })
            .collect();

        let impl_decl = ImplDecl {
            is_unsafe: false,
            generics: self.type_info.generics.iter().cloned().collect(),
            kind: ImplKind::Protocol {
                protocol: Path::single(Ident::new(protocol, span)),
                protocol_args: List::new(),
                for_type: self.type_info.as_type(span),
            },
            generic_where_clause: None,
            meta_where_clause: None,
            specialize_attr: None,
            items: items.into_iter().collect(),
            span,
        };

        Item::new(ItemKind::Impl(impl_decl), span)
    }

    /// Create a method declaration
    pub fn method(
        &self,
        name: &str,
        params: List<FunctionParam>,
        return_type: Type,
        body: Block,
        span: Span,
    ) -> FunctionDecl {
        FunctionDecl {
            visibility: Visibility::Public,
            is_async: false,
            is_pure: false, // Derive-generated methods are not pure
            is_meta: false,
            stage_level: 0,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,
            extern_abi: None,
            is_variadic: false,
            name: Ident::new(name, span),
            generics: List::new(),
            params: params.into_iter().collect(),
            return_type: Some(return_type),
            throws_clause: None,
            std_attr: None,
            contexts: List::new(),
            generic_where_clause: None,
            meta_where_clause: None,
            attributes: List::new(),
            body: Some(FunctionBody::Block(body)),
            requires: List::new(),
            ensures: List::new(),
            span,
        }
    }

    /// Create a &self parameter
    pub fn self_ref_param(&self, span: Span) -> FunctionParam {
        FunctionParam::new(FunctionParamKind::SelfRef, span)
    }

    /// Create a regular parameter
    pub fn param(&self, name: &str, ty: Type, span: Span) -> FunctionParam {
        FunctionParam::new(
            FunctionParamKind::Regular {
                pattern: Pattern::new(
                    PatternKind::Ident {
                        by_ref: false,
                        name: Ident::new(name, span),
                        mutable: false,
                        subpattern: None,
                    },
                    span,
                ),
                ty,
                default_value: verum_common::Maybe::None,
            },
            span,
        )
    }
}

/// Error types for derive macro failures
#[derive(Debug, Clone)]
pub enum DeriveError {
    /// Unknown derive macro
    UnknownDerive { name: Text, span: Span },

    /// Unsupported type kind
    UnsupportedTypeKind { kind: Text, hint: Text, span: Span },

    /// Field doesn't implement required protocol
    FieldNotImplement {
        field_name: Text,
        field_type: Type,
        protocol: Text,
        span: Span,
    },

    /// Variant pattern matching failed
    VariantPatternError {
        variant_name: Text,
        message: Text,
        span: Span,
    },

    /// Refinement type without default value
    RefinementNoDefault {
        type_name: Text,
        hint: Text,
        span: Span,
    },

    /// Internal error
    Internal { message: Text, span: Span },
}

impl std::fmt::Display for DeriveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeriveError::UnknownDerive { name, .. } => {
                write!(f, "Unknown derive macro: `{}`", name.as_str())
            }
            DeriveError::UnsupportedTypeKind { kind, hint, .. } => {
                write!(f, "Cannot derive for {}: {}", kind.as_str(), hint.as_str())
            }
            DeriveError::FieldNotImplement {
                field_name,
                protocol,
                ..
            } => {
                write!(
                    f,
                    "Field `{}` does not implement `{}`",
                    field_name.as_str(),
                    protocol.as_str()
                )
            }
            DeriveError::VariantPatternError {
                variant_name,
                message,
                ..
            } => {
                write!(
                    f,
                    "Error in variant `{}`: {}",
                    variant_name.as_str(),
                    message.as_str()
                )
            }
            DeriveError::RefinementNoDefault {
                type_name, hint, ..
            } => {
                write!(
                    f,
                    "Refinement type `{}` has no default: {}",
                    type_name.as_str(),
                    hint.as_str()
                )
            }
            DeriveError::Internal { message, .. } => {
                write!(f, "Internal derive error: {}", message.as_str())
            }
        }
    }
}

impl std::error::Error for DeriveError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_field_info() {
        let span = Span::default();
        let field = FieldInfo {
            name: Text::from("x"),
            ty: Type::int(span),
            index: 0,
            is_public: true,
            has_default: false,
            default_value: verum_common::Maybe::None,
            attributes: List::new(),
            span,
        };

        assert_eq!(field.name.as_str(), "x");
        assert_eq!(field.index, 0);
        assert!(field.is_public);
        assert!(!field.has_default);
        assert!(field.is_required());
    }

    #[test]
    fn test_variant_info() {
        let span = Span::default();
        let variant = VariantInfo {
            name: Text::from("Some"),
            fields: List::from(vec![FieldInfo {
                name: Text::from("0"),
                ty: Type::int(span),
                index: 0,
                is_public: false,
                has_default: false,
                default_value: verum_common::Maybe::None,
                attributes: List::new(),
                span,
            }]),
            is_unit: false,
            is_tuple: true,
            index: 0,
            span,
        };

        assert_eq!(variant.name.as_str(), "Some");
        assert!(!variant.is_unit);
        assert!(variant.is_tuple);
    }

    #[test]
    fn test_path_from_str() {
        let span = Span::default();
        let path = path_from_str("Foo.Bar.Baz", span);
        assert_eq!(path.segments.len(), 3);
    }
}
