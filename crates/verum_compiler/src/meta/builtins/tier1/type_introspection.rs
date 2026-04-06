//! Type Introspection (Tier 1 - Requires MetaTypes)
//!
//! Basic type introspection functions: type name, ID, kind checks, and protocol checks.
//!
//! ## Basic Type Info
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `type_name(T)` | `(Type) -> Text` | Get type name |
//! | `type_id(T)` | `(Type) -> UInt` | Get unique type identifier |
//! | `type_of(expr)` | `(Expr) -> Type` | Get type of expression |
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
//! | `is_sized(T)` | `(Type) -> Bool` | Check if Sized |
//! | `needs_drop(T)` | `(Type) -> Bool` | Check if needs destructor |
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use verum_ast::ty::{PathSegment, TypeKind};
use verum_common::{List, Text};

use crate::meta::builtins::context_requirements::{BuiltinInfo, BuiltinRegistry};
use crate::meta::builtins::type_props::{compute_type_id, compute_type_name};
use crate::meta::context::{ConstValue, MetaContext};
use crate::meta::error::MetaError;
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

/// Register type introspection builtins
pub fn register_builtins(map: &mut BuiltinRegistry) {
    // ========================================================================
    // Basic Type Info (Tier 1 - MetaTypes)
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
}

// ============================================================================
// Basic Type Info
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
        // Handle tuple of types
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
    if args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: 0,
        });
    }

    // Handle multiple args (tuple type) like type_name does
    if args.len() > 1 {
        let all_types = args.iter().all(|a| matches!(a, ConstValue::Type(_)));
        if all_types {
            // Compute a combined hash for tuple types
            let combined: u64 = args.iter().enumerate().map(|(i, a)| {
                if let ConstValue::Type(ty) = a {
                    compute_type_id(&ty.kind).wrapping_mul((i as u64).wrapping_add(1))
                } else { 0 }
            }).fold(0u64, |acc, x| acc.wrapping_add(x));
            return Ok(ConstValue::UInt(combined.into()));
        }
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            let id = compute_type_id(&ty.kind);
            Ok(ConstValue::UInt(id.into()))
        }
        // When type_of returns a text, compute ID from the name
        ConstValue::Text(name) => {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            name.as_str().hash(&mut hasher);
            Ok(ConstValue::UInt(hasher.finish().into()))
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
