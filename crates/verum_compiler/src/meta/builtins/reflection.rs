//! Type Reflection Intrinsics (Tier 1 - Requires MetaTypes)
//!
//! Provides compile-time type introspection functions that access the type registry.
//! All functions require the `MetaTypes` context since they query type information.
//!
//! ## Basic Type Introspection
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `type_name(T)` | `(Type) -> Text` | Get type name |
//! | `type_id(T)` | `(Type) -> UInt` | Get unique type identifier |
//! | `type_of(expr)` | `(Expr) -> Type` | Get type of expression |
//!
//! ## Structure Introspection
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `fields_of(T)` | `(Type) -> List<FieldInfo>` | Get struct fields |
//! | `field_access(T, name)` | `(Type, Text) -> Maybe<FieldInfo>` | Get specific field |
//! | `variants_of(T)` | `(Type) -> List<VariantInfo>` | Get enum variants |
//!
//! ## Type Kind Checks
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `is_struct(T)` | `(Type) -> Bool` | Check if record type |
//! | `is_enum(T)` | `(Type) -> Bool` | Check if sum type |
//! | `is_tuple(T)` | `(Type) -> Bool` | Check if tuple type |
//! | `kind_of(T)` | `(Type) -> TypeKind` | Get type kind |
//!
//! ## Protocol Checks
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `implements(T, P)` | `(Type, Protocol) -> Bool` | Check protocol implementation |
//! | `is_copy(T)` | `(Type) -> Bool` | Check if Copy |
//! | `is_send(T)` | `(Type) -> Bool` | Check if Send |
//! | `is_sync(T)` | `(Type) -> Bool` | Check if Sync |
//!
//! ## Context Requirements
//!
//! **Tier 1**: All functions require `using [MetaTypes]` context.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use verum_ast::ty::{PathSegment, TypeKind};
use verum_common::{Heap, List, Maybe, Text};

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};
use super::type_props::{compute_type_id, compute_type_name};
use super::{ConstValue, MetaContext, MetaError};
use crate::meta::TypeKind as MetaTypeKind;

/// Helper function to extract the name from a PathSegment
fn segment_name(segment: &PathSegment) -> Option<Text> {
    match segment {
        PathSegment::Name(ident) => Some(ident.name.clone()),
        PathSegment::SelfValue => Some(Text::from("self")),
        PathSegment::Super => Some(Text::from("super")),
        PathSegment::Cog => Some(Text::from("cog")),
        PathSegment::Relative => None,
    }
}

/// Helper function to extract the name as &str from a PathSegment
fn segment_name_str(segment: &PathSegment) -> Option<&str> {
    match segment {
        PathSegment::Name(ident) => Some(ident.name.as_str()),
        PathSegment::SelfValue => Some("self"),
        PathSegment::Super => Some("super"),
        PathSegment::Cog => Some("cog"),
        PathSegment::Relative => None,
    }
}

/// Register reflection builtins with context requirements
///
/// All reflection functions require MetaTypes context since they access
/// type information from the type registry.
pub fn register_builtins(map: &mut BuiltinRegistry) {
    // ========================================================================
    // Basic Type Introspection (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("type_name"),
        BuiltinInfo::meta_types(
            meta_type_name,
            "Get the name of a type as text",
            "(Type) -> Text",
        ),
    );
    map.insert(
        Text::from("type_id"),
        BuiltinInfo::meta_types(
            meta_type_id,
            "Get unique type identifier",
            "(Type) -> UInt",
        ),
    );
    map.insert(
        Text::from("type_of"),
        BuiltinInfo::meta_types(
            meta_type_of,
            "Get the type of an expression",
            "(Expr) -> Type",
        ),
    );

    // ========================================================================
    // Structure Introspection (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("fields_of"),
        BuiltinInfo::meta_types(
            meta_fields_of,
            "Get list of struct/record fields",
            "(Type) -> List<FieldInfo>",
        ),
    );
    map.insert(
        Text::from("type_fields"),
        BuiltinInfo::meta_types(
            meta_fields_of,
            "Get list of type fields (alias for fields_of)",
            "(Type) -> List<FieldInfo>",
        ),
    );
    map.insert(
        Text::from("field_access"),
        BuiltinInfo::meta_types(
            meta_field_access,
            "Get specific field by name",
            "(Type, Text) -> Maybe<FieldInfo>",
        ),
    );
    map.insert(
        Text::from("variants_of"),
        BuiltinInfo::meta_types(
            meta_variants_of,
            "Get list of enum/sum type variants",
            "(Type) -> List<VariantInfo>",
        ),
    );

    // ========================================================================
    // Type Kind Checks (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("is_struct"),
        BuiltinInfo::meta_types(
            meta_is_struct,
            "Check if type is a record/struct",
            "(Type) -> Bool",
        ),
    );
    map.insert(
        Text::from("is_enum"),
        BuiltinInfo::meta_types(
            meta_is_enum,
            "Check if type is a sum/enum type",
            "(Type) -> Bool",
        ),
    );
    map.insert(
        Text::from("is_tuple"),
        BuiltinInfo::meta_types(
            meta_is_tuple,
            "Check if type is a tuple",
            "(Type) -> Bool",
        ),
    );
    map.insert(
        Text::from("kind_of"),
        BuiltinInfo::meta_types(
            meta_kind_of,
            "Get the kind classification of a type",
            "(Type) -> TypeKind",
        ),
    );

    // ========================================================================
    // Protocol/Trait Checks (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("implements"),
        BuiltinInfo::meta_types(
            meta_implements,
            "Check if type implements a protocol",
            "(Type, Protocol) -> Bool",
        ),
    );
    map.insert(
        Text::from("is_copy"),
        BuiltinInfo::meta_types(
            meta_is_copy,
            "Check if type implements Copy",
            "(Type) -> Bool",
        ),
    );
    map.insert(
        Text::from("is_send"),
        BuiltinInfo::meta_types(
            meta_is_send,
            "Check if type implements Send",
            "(Type) -> Bool",
        ),
    );
    map.insert(
        Text::from("is_sync"),
        BuiltinInfo::meta_types(
            meta_is_sync,
            "Check if type implements Sync",
            "(Type) -> Bool",
        ),
    );
    map.insert(
        Text::from("is_sized"),
        BuiltinInfo::meta_types(
            meta_is_sized,
            "Check if type is Sized",
            "(Type) -> Bool",
        ),
    );
    map.insert(
        Text::from("needs_drop"),
        BuiltinInfo::meta_types(
            meta_needs_drop,
            "Check if type needs destructor",
            "(Type) -> Bool",
        ),
    );

    // ========================================================================
    // Extended Type Info (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("simple_name_of"),
        BuiltinInfo::meta_types(
            meta_simple_name_of,
            "Get simple (unqualified) type name",
            "(Type) -> Text",
        ),
    );
    map.insert(
        Text::from("module_of"),
        BuiltinInfo::meta_types(
            meta_module_of,
            "Get module path where type is defined",
            "(Type) -> Text",
        ),
    );
    map.insert(
        Text::from("generics_of"),
        BuiltinInfo::meta_types(
            meta_generics_of,
            "Get generic parameters of a type",
            "(Type) -> List<GenericParam>",
        ),
    );
    map.insert(
        Text::from("protocols_of"),
        BuiltinInfo::meta_types(
            meta_protocols_of,
            "Get list of protocols implemented by type",
            "(Type) -> List<Protocol>",
        ),
    );
    map.insert(
        Text::from("inner_type_of"),
        BuiltinInfo::meta_types(
            meta_inner_type_of,
            "Get inner type (e.g., T from Maybe<T>)",
            "(Type) -> Maybe<Type>",
        ),
    );
    map.insert(
        Text::from("element_type_of"),
        BuiltinInfo::meta_types(
            meta_element_type_of,
            "Get element type of collection",
            "(Type) -> Maybe<Type>",
        ),
    );

    // ========================================================================
    // Function/Method Introspection (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("functions_of"),
        BuiltinInfo::meta_types(
            meta_functions_of,
            "Get all functions defined on a type",
            "(Type) -> List<FunctionInfo>",
        ),
    );
    map.insert(
        Text::from("method_of"),
        BuiltinInfo::meta_types(
            meta_method_of,
            "Get specific method by name",
            "(Type, Text) -> Maybe<FunctionInfo>",
        ),
    );
    map.insert(
        Text::from("static_functions_of"),
        BuiltinInfo::meta_types(
            meta_static_functions_of,
            "Get static (associated) functions",
            "(Type) -> List<FunctionInfo>",
        ),
    );
    map.insert(
        Text::from("instance_methods_of"),
        BuiltinInfo::meta_types(
            meta_instance_methods_of,
            "Get instance methods (with self)",
            "(Type) -> List<FunctionInfo>",
        ),
    );

    // ========================================================================
    // Constraint Introspection (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("bounds_of"),
        BuiltinInfo::meta_types(
            meta_bounds_of,
            "Get trait bounds of a type parameter",
            "(Type) -> List<Bound>",
        ),
    );
    map.insert(
        Text::from("lifetime_params_of"),
        BuiltinInfo::meta_types(
            meta_lifetime_params_of,
            "Get lifetime parameters of a type",
            "(Type) -> List<Lifetime>",
        ),
    );
    map.insert(
        Text::from("where_clause_of"),
        BuiltinInfo::meta_types(
            meta_where_clause_of,
            "Get where clause constraints",
            "(Type) -> List<Constraint>",
        ),
    );

    // ========================================================================
    // Memory Layout Introspection (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("field_offset"),
        BuiltinInfo::meta_types(
            meta_field_offset,
            "Get byte offset of a field",
            "(Type, Text) -> Int",
        ),
    );
    map.insert(
        Text::from("memory_layout_of"),
        BuiltinInfo::meta_types(
            meta_memory_layout_of,
            "Get complete memory layout info",
            "(Type) -> LayoutInfo",
        ),
    );
    map.insert(
        Text::from("ownership_of"),
        BuiltinInfo::meta_types(
            meta_ownership_of,
            "Get ownership semantics of type",
            "(Type) -> OwnershipInfo",
        ),
    );

    // ========================================================================
    // Attribute Introspection (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("attributes_of"),
        BuiltinInfo::meta_types(
            meta_attributes_of,
            "Get all attributes of a type",
            "(Type) -> List<Attribute>",
        ),
    );
    map.insert(
        Text::from("has_attribute"),
        BuiltinInfo::meta_types(
            meta_has_attribute,
            "Check if type has specific attribute",
            "(Type, Text) -> Bool",
        ),
    );
    map.insert(
        Text::from("get_attribute"),
        BuiltinInfo::meta_types(
            meta_get_attribute,
            "Get attribute value by name",
            "(Type, Text) -> Maybe<Attribute>",
        ),
    );
    map.insert(
        Text::from("doc_of"),
        BuiltinInfo::meta_types(
            meta_doc_of,
            "Get documentation string of type",
            "(Type) -> Maybe<Text>",
        ),
    );

    // ========================================================================
    // Associated Types (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("associated_types_of"),
        BuiltinInfo::meta_types(
            meta_associated_types_of,
            "Get associated types of a type",
            "(Type) -> List<AssociatedType>",
        ),
    );
    map.insert(
        Text::from("super_types_of"),
        BuiltinInfo::meta_types(
            meta_super_types_of,
            "Get super types (parent protocols)",
            "(Type) -> List<Type>",
        ),
    );
    map.insert(
        Text::from("key_value_types_of"),
        BuiltinInfo::meta_types(
            meta_key_value_types_of,
            "Get key and value types for map-like types",
            "(Type) -> Maybe<(Type, Type)>",
        ),
    );
}

// ============================================================================
// Basic Type Introspection
// ============================================================================

/// Get type name as Text
fn meta_type_name(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: 0,
        });
    }

    // Multiple arguments means a tuple type: type_name(Int, Text) -> "(Int, Text)"
    // This happens because type_name((Int, Text)) parses as type_name(Int, Text)
    if args.len() > 1 {
        let all_types = args.iter().all(|a| matches!(a, ConstValue::Type(_)));
        if all_types {
            let names: Vec<String> = args.iter().map(|a| {
                if let ConstValue::Type(ty) = a {
                    compute_type_name(&ty.kind).to_string()
                } else {
                    "unknown".to_string()
                }
            }).collect();
            return Ok(ConstValue::Text(Text::from(format!("({})", names.join(", ")))));
        }
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            let name = compute_type_name(&ty.kind);
            Ok(ConstValue::Text(name))
        }
        // Handle tuple of types: type_name((Int, Text)) when parsed as tuple ConstValue
        ConstValue::Tuple(elements) => {
            let names: Vec<String> = elements.iter().map(|e| {
                if let ConstValue::Type(ty) = e {
                    compute_type_name(&ty.kind).to_string()
                } else {
                    "unknown".to_string()
                }
            }).collect();
            Ok(ConstValue::Text(Text::from(format!("({})", names.join(", ")))))
        }
        // Handle Expr that might represent a type
        ConstValue::Expr(expr) => {
            if let verum_ast::ExprKind::TypeExpr(ty) = &expr.kind {
                let name = compute_type_name(&ty.kind);
                Ok(ConstValue::Text(name))
            } else {
                Err(MetaError::TypeMismatch {
                    expected: Text::from("Type"),
                    found: args[0].type_name(),
                })
            }
        }
        // When type_of returns a text (the type name), just pass it through
        ConstValue::Text(name) => Ok(ConstValue::Text(name.clone())),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get unique type identifier
fn meta_type_id(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            let id = compute_type_id(&ty.kind);
            Ok(ConstValue::UInt(id.into()))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get the type of an expression (placeholder)
fn meta_type_of(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    // type_of is typically handled specially by the evaluator
    // This fallback returns the value's inherent type
    Ok(ConstValue::Text(args[0].type_name()))
}

// ============================================================================
// Structure Introspection
// ============================================================================

/// Get struct fields as List<(Text, Type)>
fn meta_fields_of(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            // Try to get type name from the type
            if let TypeKind::Path(path) = &ty.kind {
                if let Some(first) = path.segments.first() {
                    let type_name = segment_name(first).unwrap_or_else(|| Text::from("unknown"));
                    if let Some(fields) = ctx.get_struct_fields(&type_name) {
                        let field_list: List<ConstValue> = fields
                            .iter()
                            .map(|(name, field_ty)| {
                                ConstValue::Tuple(List::from(vec![
                                    ConstValue::Text(name.clone()),
                                    ConstValue::Type(field_ty.clone()),
                                ]))
                            })
                            .collect();
                        return Ok(ConstValue::Array(field_list));
                    }
                }
            }

            // For tuple types, return indexed fields
            if let TypeKind::Tuple(elements) = &ty.kind {
                let field_list: List<ConstValue> = elements
                    .iter()
                    .enumerate()
                    .map(|(i, field_ty)| {
                        ConstValue::Tuple(List::from(vec![
                            ConstValue::Text(Text::from(format!("{}", i))),
                            ConstValue::Type(field_ty.clone()),
                        ]))
                    })
                    .collect();
                return Ok(ConstValue::Array(field_list));
            }

            Ok(ConstValue::Array(List::new()))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Access a field by name at compile-time
fn meta_field_access(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Tuple(fields), ConstValue::Int(index)) => {
            let idx = *index as usize;
            if idx < fields.len() {
                Ok(fields[idx].clone())
            } else {
                Err(MetaError::Other(Text::from(format!(
                    "Tuple index {} out of bounds (len={})",
                    idx,
                    fields.len()
                ))))
            }
        }
        (ConstValue::Tuple(_), ConstValue::Text(name)) => {
            // For named tuple access (if supported)
            Err(MetaError::Other(Text::from(format!(
                "Named field access '{}' not supported for tuples",
                name
            ))))
        }
        _ => Err(MetaError::Other(Text::from(
            "field_access requires (value, index) or (value, field_name)",
        ))),
    }
}

/// Get enum variants as List<Text>
fn meta_variants_of(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            if let TypeKind::Path(path) = &ty.kind {
                if let Some(first) = path.segments.first() {
                    let type_name = segment_name(first).unwrap_or_else(|| Text::from("unknown"));
                    if let Some(variants) = ctx.get_enum_variants(&type_name) {
                        let variant_list: List<ConstValue> = variants
                            .iter()
                            .map(|(name, _)| ConstValue::Text(name.clone()))
                            .collect();
                        return Ok(ConstValue::Array(variant_list));
                    }
                }
            }
            Ok(ConstValue::Array(List::new()))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

// ============================================================================
// Type Kind Checks
// ============================================================================

/// Check if type is a struct
fn meta_is_struct(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            if let TypeKind::Path(path) = &ty.kind {
                if let Some(first) = path.segments.first() {
                    let type_name = segment_name(first).unwrap_or_else(|| Text::from("unknown"));
                    if let Some(def) = ctx.get_type_definition(&type_name) {
                        return Ok(ConstValue::Bool(matches!(
                            def,
                            crate::meta::TypeDefinition::Struct { .. }
                        )));
                    }
                }
            }
            Ok(ConstValue::Bool(false))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Check if type is an enum
fn meta_is_enum(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            if let TypeKind::Path(path) = &ty.kind {
                if let Some(first) = path.segments.first() {
                    let type_name = segment_name(first).unwrap_or_else(|| Text::from("unknown"));
                    if let Some(def) = ctx.get_type_definition(&type_name) {
                        return Ok(ConstValue::Bool(matches!(
                            def,
                            crate::meta::TypeDefinition::Enum { .. }
                        )));
                    }
                }
            }
            Ok(ConstValue::Bool(false))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Check if type is a tuple
fn meta_is_tuple(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => Ok(ConstValue::Bool(matches!(ty.kind, TypeKind::Tuple(_)))),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get type kind classification
fn meta_kind_of(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            let kind = match &ty.kind {
                TypeKind::Tuple(_) => MetaTypeKind::Tuple,
                TypeKind::Array { .. } => MetaTypeKind::Array,
                TypeKind::Slice { .. } => MetaTypeKind::Slice,
                TypeKind::Reference { .. }
                | TypeKind::CheckedReference { .. }
                | TypeKind::UnsafeReference { .. } => MetaTypeKind::Reference,
                TypeKind::Pointer { .. } => MetaTypeKind::Pointer,
                TypeKind::Function { .. } => MetaTypeKind::Function,
                TypeKind::Never => MetaTypeKind::Never,
                TypeKind::Unit => MetaTypeKind::Unit,
                TypeKind::Unknown => MetaTypeKind::Infer,
                TypeKind::Bool | TypeKind::Char | TypeKind::Int | TypeKind::Float | TypeKind::Text => {
                    MetaTypeKind::Primitive
                }
                TypeKind::Path(path) => {
                    // Look up in type definitions
                    if let Some(first) = path.segments.first() {
                        let type_name = segment_name(first).unwrap_or_else(|| Text::from("unknown"));
                        if let Some(def) = ctx.get_type_definition(&type_name) {
                            match def {
                                crate::meta::TypeDefinition::Struct { .. } => MetaTypeKind::Struct,
                                crate::meta::TypeDefinition::Enum { .. } => MetaTypeKind::Enum,
                                crate::meta::TypeDefinition::Protocol { .. } => MetaTypeKind::Protocol,
                                crate::meta::TypeDefinition::Alias { .. } => MetaTypeKind::Alias,
                                crate::meta::TypeDefinition::Newtype { .. } => MetaTypeKind::Newtype,
                            }
                        } else {
                            MetaTypeKind::Unknown
                        }
                    } else {
                        MetaTypeKind::Unknown
                    }
                }
                _ => MetaTypeKind::Unknown,
            };
            Ok(kind.to_meta_value())
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

// ============================================================================
// Protocol/Trait Checks
// ============================================================================

/// Check if type implements a protocol
fn meta_implements(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Type(ty), ConstValue::Type(protocol)) => {
            let type_name = compute_type_name(&ty.kind);
            let protocol_name = compute_type_name(&protocol.kind);
            Ok(ConstValue::Bool(
                ctx.type_implements_protocol(&type_name, &protocol_name),
            ))
        }
        (ConstValue::Type(ty), ConstValue::Text(protocol_name)) => {
            let type_name = compute_type_name(&ty.kind);
            Ok(ConstValue::Bool(
                ctx.type_implements_protocol(&type_name, protocol_name),
            ))
        }
        _ => Err(MetaError::Other(Text::from(
            "implements requires (Type, Type) or (Type, Text)",
        ))),
    }
}

/// Check if type is Copy (placeholder - would need full type analysis)
fn meta_is_copy(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            // Primitives are Copy
            let is_copy = matches!(
                ty.kind,
                TypeKind::Bool
                    | TypeKind::Char
                    | TypeKind::Int
                    | TypeKind::Float
                    | TypeKind::Unit
                    | TypeKind::Never
            );
            Ok(ConstValue::Bool(is_copy))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Check if type is Send
fn meta_is_send(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }
    // Most types are Send by default (simplified)
    Ok(ConstValue::Bool(true))
}

/// Check if type is Sync
fn meta_is_sync(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }
    // Most types are Sync by default (simplified)
    Ok(ConstValue::Bool(true))
}

/// Check if type is Sized
fn meta_is_sized(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            // Slices and trait objects are not sized
            let is_sized = !matches!(ty.kind, TypeKind::Slice { .. });
            Ok(ConstValue::Bool(is_sized))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Check if type needs drop
fn meta_needs_drop(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            // Primitives don't need drop
            let needs_drop = !matches!(
                ty.kind,
                TypeKind::Bool
                    | TypeKind::Char
                    | TypeKind::Int
                    | TypeKind::Float
                    | TypeKind::Unit
                    | TypeKind::Never
            );
            Ok(ConstValue::Bool(needs_drop))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

// ============================================================================
// Extended Type Info
// ============================================================================

/// Get type's simple (unqualified) name
fn meta_simple_name_of(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            if let TypeKind::Path(path) = &ty.kind {
                if let Some(last) = path.segments.last() {
                    if let Some(name) = segment_name_str(last) {
                        return Ok(ConstValue::Text(Text::from(name)));
                    }
                }
            }
            Ok(ConstValue::Text(compute_type_name(&ty.kind)))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get type's module path
fn meta_module_of(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            if let TypeKind::Path(path) = &ty.kind {
                if path.segments.len() > 1 {
                    let module_parts: Vec<&str> = path.segments[..path.segments.len() - 1]
                        .iter()
                        .filter_map(segment_name_str)
                        .collect();
                    return Ok(ConstValue::Text(Text::from(module_parts.join("::"))));
                }
            }
            Ok(ConstValue::Text(Text::from("")))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get generic type parameters
fn meta_generics_of(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            // Handle Generic types (e.g., List<T>, Option<T>)
            if let TypeKind::Generic { args: type_args, .. } = &ty.kind {
                let generics: Vec<ConstValue> = type_args
                    .iter()
                    .filter_map(|arg| {
                        if let verum_ast::ty::GenericArg::Type(t) = arg {
                            Some(ConstValue::Type(t.clone()))
                        } else {
                            None
                        }
                    })
                    .collect();
                return Ok(ConstValue::Array(List::from(generics)));
            }
            Ok(ConstValue::Array(List::new()))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get implemented protocols
fn meta_protocols_of(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            let type_name = compute_type_name(&ty.kind);
            let protocols = ctx.get_implemented_protocols(&type_name);
            let result: List<ConstValue> = protocols.into_iter().map(ConstValue::Text).collect();
            Ok(ConstValue::Array(result))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get inner type for wrapper types
fn meta_inner_type_of(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            match &ty.kind {
                TypeKind::Reference { inner, .. }
                | TypeKind::CheckedReference { inner, .. }
                | TypeKind::UnsafeReference { inner, .. } => {
                    let inner_ty = verum_ast::ty::Type {
                        kind: inner.kind.clone(),
                        span: verum_ast::Span::dummy(),
                    };
                    Ok(ConstValue::Maybe(Maybe::Some(Heap::new(ConstValue::Type(
                        inner_ty,
                    )))))
                }
                TypeKind::Pointer { inner, .. } => {
                    let inner_ty = verum_ast::ty::Type {
                        kind: inner.kind.clone(),
                        span: verum_ast::Span::dummy(),
                    };
                    Ok(ConstValue::Maybe(Maybe::Some(Heap::new(ConstValue::Type(
                        inner_ty,
                    )))))
                }
                TypeKind::Array { element, .. } | TypeKind::Slice(element) => {
                    let inner_ty = verum_ast::ty::Type {
                        kind: element.kind.clone(),
                        span: verum_ast::Span::dummy(),
                    };
                    Ok(ConstValue::Maybe(Maybe::Some(Heap::new(ConstValue::Type(
                        inner_ty,
                    )))))
                }
                _ => Ok(ConstValue::Maybe(Maybe::None)),
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get element type for collections
fn meta_element_type_of(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            match &ty.kind {
                TypeKind::Array { element, .. } | TypeKind::Slice(element) => {
                    let elem_ty = verum_ast::ty::Type {
                        kind: element.kind.clone(),
                        span: verum_ast::Span::dummy(),
                    };
                    Ok(ConstValue::Maybe(Maybe::Some(Heap::new(ConstValue::Type(
                        elem_ty,
                    )))))
                }
                TypeKind::Tuple(elements) if !elements.is_empty() => {
                    let first_ty = elements[0].clone();
                    Ok(ConstValue::Maybe(Maybe::Some(Heap::new(ConstValue::Type(
                        first_ty,
                    )))))
                }
                _ => Ok(ConstValue::Maybe(Maybe::None)),
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

// ============================================================================
// Function/Method Introspection
// ============================================================================

/// Get all functions/methods of a type
fn meta_functions_of(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            let type_name = compute_type_name(&ty.kind);
            let functions = ctx.get_type_functions(&type_name);
            let result: List<ConstValue> = functions
                .into_iter()
                .map(|f| ConstValue::Text(f.name))
                .collect();
            Ok(ConstValue::Array(result))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get a specific method by name
fn meta_method_of(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Type(ty), ConstValue::Text(method_name)) => {
            let type_name = compute_type_name(&ty.kind);
            if let Some(resolution) = ctx.resolve_method(&type_name, method_name) {
                Ok(ConstValue::Maybe(Maybe::Some(Heap::new(ConstValue::Text(
                    resolution.function.name,
                )))))
            } else {
                Ok(ConstValue::Maybe(Maybe::None))
            }
        }
        _ => Err(MetaError::Other(Text::from(
            "method_of requires (Type, Text)",
        ))),
    }
}

/// Get static functions of a type
fn meta_static_functions_of(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    // For now, same as functions_of (would need more metadata to distinguish)
    meta_functions_of(ctx, args)
}

/// Get instance methods of a type
fn meta_instance_methods_of(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    // For now, same as functions_of (would need more metadata to distinguish)
    meta_functions_of(ctx, args)
}

// ============================================================================
// Constraint Introspection
// ============================================================================

/// Get protocol bounds
fn meta_bounds_of(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }
    // Placeholder - would need full type analysis
    Ok(ConstValue::Array(List::new()))
}

/// Get lifetime parameters
fn meta_lifetime_params_of(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }
    // Placeholder - would need full type analysis
    Ok(ConstValue::Array(List::new()))
}

/// Get where clause constraints
fn meta_where_clause_of(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }
    // Placeholder - would need full type analysis
    Ok(ConstValue::Array(List::new()))
}

// ============================================================================
// Memory Layout Introspection
// ============================================================================

/// Get field offset
fn meta_field_offset(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }
    // Placeholder - would need full type layout analysis
    Ok(ConstValue::Maybe(Maybe::None))
}

/// Get memory layout
fn meta_memory_layout_of(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }
    // Placeholder - would need full type layout analysis
    Ok(ConstValue::Array(List::new()))
}

/// Get ownership info
fn meta_ownership_of(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }
    // Placeholder
    Ok(ConstValue::Unit)
}

// ============================================================================
// Attribute Introspection
// ============================================================================

/// Get type attributes
fn meta_attributes_of(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            let type_name = compute_type_name(&ty.kind);
            let attrs = ctx.get_type_attributes(&type_name);
            let result: List<ConstValue> = attrs.into_iter().map(ConstValue::Text).collect();
            Ok(ConstValue::Array(result))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Check if type has specific attribute
fn meta_has_attribute(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Type(ty), ConstValue::Text(attr_name)) => {
            let type_name = compute_type_name(&ty.kind);
            Ok(ConstValue::Bool(
                ctx.type_has_attribute(&type_name, attr_name),
            ))
        }
        _ => Err(MetaError::Other(Text::from(
            "has_attribute requires (Type, Text)",
        ))),
    }
}

/// Get attribute value
fn meta_get_attribute(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Type(ty), ConstValue::Text(attr_name)) => {
            let type_name = compute_type_name(&ty.kind);
            match ctx.get_type_attribute(&type_name, attr_name) {
                Some(val) => Ok(ConstValue::Maybe(Maybe::Some(Heap::new(val)))),
                None => Ok(ConstValue::Maybe(Maybe::None)),
            }
        }
        _ => Err(MetaError::Other(Text::from(
            "get_attribute requires (Type, Text)",
        ))),
    }
}

/// Get documentation
fn meta_doc_of(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            let type_name = compute_type_name(&ty.kind);
            match ctx.get_type_doc(&type_name) {
                Maybe::Some(doc) => Ok(ConstValue::Maybe(Maybe::Some(Heap::new(
                    ConstValue::Text(doc),
                )))),
                Maybe::None => Ok(ConstValue::Maybe(Maybe::None)),
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

// ============================================================================
// Associated Types
// ============================================================================

/// Get associated types
fn meta_associated_types_of(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            let type_name = compute_type_name(&ty.kind);
            let assoc_types = ctx.get_associated_types(&type_name);
            let result: List<ConstValue> = assoc_types
                .into_iter()
                .map(|(name, _ty)| ConstValue::Text(name))
                .collect();
            Ok(ConstValue::Array(result))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get super types
fn meta_super_types_of(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            let type_name = compute_type_name(&ty.kind);
            let super_types = ctx.get_super_types(&type_name);
            let result: List<ConstValue> = super_types.into_iter().map(ConstValue::Text).collect();
            Ok(ConstValue::Array(result))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get key/value types for map-like types
fn meta_key_value_types_of(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            // Check if it's a Generic type like Map<K, V>
            if let TypeKind::Generic { base, args: type_args } = &ty.kind {
                // Check if base is a Map type
                if let TypeKind::Path(path) = &base.kind {
                    if let Some(first) = path.segments.first() {
                        if let Some(name) = segment_name_str(first) {
                            if matches!(name, verum_common::well_known_types::type_names::MAP | "HashMap" | "BTreeMap") {
                                // Extract key and value types from generic args
                                let types: Vec<verum_ast::ty::Type> = type_args
                                    .iter()
                                    .filter_map(|arg| {
                                        if let verum_ast::ty::GenericArg::Type(t) = arg {
                                            Some(t.clone())
                                        } else {
                                            None
                                        }
                                    })
                                    .collect();
                                if types.len() >= 2 {
                                    return Ok(ConstValue::Maybe(Maybe::Some(Heap::new(
                                        ConstValue::Tuple(List::from(vec![
                                            ConstValue::Type(types[0].clone()),
                                            ConstValue::Type(types[1].clone()),
                                        ])),
                                    ))));
                                }
                            }
                        }
                    }
                }
            }
            Ok(ConstValue::Maybe(Maybe::None))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::Span;

    #[test]
    fn test_type_name() {
        let mut ctx = MetaContext::new();
        let ty = verum_ast::ty::Type::int(Span::dummy());
        let args = List::from(vec![ConstValue::Type(ty)]);
        let result = meta_type_name(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Text(Text::from("Int")));
    }

    #[test]
    fn test_is_tuple() {
        let mut ctx = MetaContext::new();
        let ty = verum_ast::ty::Type::new(
            TypeKind::Tuple(List::from(vec![
                verum_ast::ty::Type::int(Span::dummy()),
                verum_ast::ty::Type::bool(Span::dummy()),
            ])),
            Span::dummy(),
        );
        let args = List::from(vec![ConstValue::Type(ty)]);
        let result = meta_is_tuple(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));
    }

    #[test]
    fn test_is_copy_primitives() {
        let mut ctx = MetaContext::new();
        let ty = verum_ast::ty::Type::int(Span::dummy());
        let args = List::from(vec![ConstValue::Type(ty)]);
        let result = meta_is_copy(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));
    }
}
