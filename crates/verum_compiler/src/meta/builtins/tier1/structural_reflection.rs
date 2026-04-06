//! Structural Reflection (Tier 1 - Requires MetaTypes)
//!
//! Reflection over type structure: fields, variants, methods, attributes.
//!
//! ## Structure Introspection
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `fields_of(T)` | `(Type) -> List<FieldInfo>` | Get struct fields |
//! | `field_access(T, name)` | `(Type, Text) -> Maybe<FieldInfo>` | Get specific field |
//! | `variants_of(T)` | `(Type) -> List<VariantInfo>` | Get enum variants |
//!
//! ## Extended Type Info
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `simple_name_of(T)` | `(Type) -> Text` | Get unqualified name |
//! | `module_of(T)` | `(Type) -> Text` | Get module path |
//! | `generics_of(T)` | `(Type) -> List<GenericParam>` | Get generic parameters |
//! | `protocols_of(T)` | `(Type) -> List<Protocol>` | Get implemented protocols |
//! | `inner_type_of(T)` | `(Type) -> Maybe<Type>` | Get inner type |
//! | `element_type_of(T)` | `(Type) -> Maybe<Type>` | Get element type |
//!
//! ## Function/Method Introspection
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `functions_of(T)` | `(Type) -> List<FunctionInfo>` | Get all functions |
//! | `method_of(T, name)` | `(Type, Text) -> Maybe<FunctionInfo>` | Get specific method |
//! | `static_functions_of(T)` | `(Type) -> List<FunctionInfo>` | Get static functions |
//! | `instance_methods_of(T)` | `(Type) -> List<FunctionInfo>` | Get instance methods |
//!
//! ## Memory Layout
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `field_offset(T, name)` | `(Type, Text) -> Int` | Get byte offset |
//! | `memory_layout_of(T)` | `(Type) -> LayoutInfo` | Get layout info |
//! | `ownership_of(T)` | `(Type) -> OwnershipInfo` | Get ownership semantics |
//!
//! ## Attribute Introspection
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `attributes_of(T)` | `(Type) -> List<Attribute>` | Get all attributes |
//! | `has_attribute(T, name)` | `(Type, Text) -> Bool` | Check for attribute |
//! | `get_attribute(T, name)` | `(Type, Text) -> Maybe<Attribute>` | Get attribute value |
//! | `doc_of(T)` | `(Type) -> Maybe<Text>` | Get documentation |
//!
//! ## Associated Types
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `associated_types_of(T)` | `(Type) -> List<AssociatedType>` | Get associated types |
//! | `super_types_of(T)` | `(Type) -> List<Type>` | Get super types |
//! | `key_value_types_of(T)` | `(Type) -> Maybe<(Type, Type)>` | Get map key/value types |
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use verum_ast::ty::{PathSegment, TypeKind};
use verum_common::{Heap, List, Maybe, Text};

use crate::meta::builtins::context_requirements::{BuiltinInfo, BuiltinRegistry};
use crate::meta::builtins::type_props::compute_type_name;
use crate::meta::context::{ConstValue, MetaContext};
use crate::meta::error::MetaError;

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

/// Register structural reflection builtins
pub fn register_builtins(map: &mut BuiltinRegistry) {
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

                    // Check if type exists in the registry
                    if let Some(type_def) = ctx.get_type_definition(&type_name) {
                        use crate::meta::TypeDefinition;
                        match type_def {
                            TypeDefinition::Struct { fields, .. } => {
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
                            TypeDefinition::Enum { .. } => {
                                return Err(MetaError::TypeMismatch {
                                    expected: Text::from("struct/record type"),
                                    found: Text::from("enum/variant type"),
                                });
                            }
                            TypeDefinition::Protocol { .. } => {
                                return Err(MetaError::TypeMismatch {
                                    expected: Text::from("struct/record type"),
                                    found: Text::from("protocol type"),
                                });
                            }
                            TypeDefinition::Alias { .. } => {
                                return Err(MetaError::TypeMismatch {
                                    expected: Text::from("struct/record type"),
                                    found: Text::from("type alias"),
                                });
                            }
                            TypeDefinition::Newtype { .. } => {
                                return Err(MetaError::TypeMismatch {
                                    expected: Text::from("struct/record type"),
                                    found: Text::from("newtype"),
                                });
                            }
                        }
                    }
                    // Type not in registry - return empty (unknown type)
                    return Ok(ConstValue::Array(List::new()));
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

                    // Check if type exists in the registry
                    if let Some(type_def) = ctx.get_type_definition(&type_name) {
                        use crate::meta::TypeDefinition;
                        match type_def {
                            TypeDefinition::Enum { variants, .. } => {
                                let variant_list: List<ConstValue> = variants
                                    .iter()
                                    .map(|(name, _)| ConstValue::Text(name.clone()))
                                    .collect();
                                return Ok(ConstValue::Array(variant_list));
                            }
                            TypeDefinition::Struct { .. } => {
                                return Err(MetaError::TypeMismatch {
                                    expected: Text::from("enum/variant type"),
                                    found: Text::from("struct/record type"),
                                });
                            }
                            TypeDefinition::Protocol { .. } => {
                                return Err(MetaError::TypeMismatch {
                                    expected: Text::from("enum/variant type"),
                                    found: Text::from("protocol type"),
                                });
                            }
                            TypeDefinition::Alias { .. } => {
                                return Err(MetaError::TypeMismatch {
                                    expected: Text::from("enum/variant type"),
                                    found: Text::from("type alias"),
                                });
                            }
                            TypeDefinition::Newtype { .. } => {
                                return Err(MetaError::TypeMismatch {
                                    expected: Text::from("enum/variant type"),
                                    found: Text::from("newtype"),
                                });
                            }
                        }
                    }
                    // Type not in registry - return empty (unknown type)
                    return Ok(ConstValue::Array(List::new()));
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
