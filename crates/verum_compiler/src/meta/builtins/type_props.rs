//! Type Property Intrinsics (Tier 1 - Requires MetaTypes)
//!
//! Provides compile-time type property functions that access type layout information.
//! All functions require the `MetaTypes` context since they query the type registry.
//!
//! ## Functions
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `size_of(T)` | `(Type) -> Int` | Get type size in bytes |
//! | `align_of(T)` | `(Type) -> Int` | Get type alignment in bytes |
//! | `stride_of(T)` | `(Type) -> Int` | Get array element stride |
//! | `type_bits(T)` | `(Type) -> Int` | Get type size in bits |
//! | `type_min(T)` | `(Type) -> T` | Get minimum value for numeric type |
//! | `type_max(T)` | `(Type) -> T` | Get maximum value for numeric type |
//!
//! ## Type Properties Syntax (Preferred)
//!
//! New code should use **Type Properties** syntax where available:
//!   - `T.size` instead of `size_of(T)`
//!   - `T.alignment` instead of `align_of(T)`
//!   - `T.stride` instead of `stride_of(T)`
//!   - `T.bits` instead of `type_bits(T)`
//!   - `T.min` instead of `type_min(T)`
//!   - `T.max` instead of `type_max(T)`
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

use verum_ast::ty::TypeKind;
use verum_common::{List, Text};

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};
use super::{ConstValue, MetaContext, MetaError};

/// Register type property builtins with context requirements
///
/// All type property functions require MetaTypes context since they
/// access type layout information from the type registry.
pub fn register_builtins(map: &mut BuiltinRegistry) {
    map.insert(
        Text::from("size_of"),
        BuiltinInfo::meta_types(
            meta_size_of,
            "Get type size in bytes",
            "(Type) -> Int",
        ),
    );
    map.insert(
        Text::from("align_of"),
        BuiltinInfo::meta_types(
            meta_align_of,
            "Get type alignment in bytes",
            "(Type) -> Int",
        ),
    );
    map.insert(
        Text::from("stride_of"),
        BuiltinInfo::meta_types(
            meta_stride_of,
            "Get array element stride in bytes",
            "(Type) -> Int",
        ),
    );
    map.insert(
        Text::from("type_bits"),
        BuiltinInfo::meta_types(
            meta_type_bits,
            "Get type size in bits",
            "(Type) -> Int",
        ),
    );
    map.insert(
        Text::from("type_min"),
        BuiltinInfo::meta_types(
            meta_type_min,
            "Get minimum value for numeric type",
            "(Type) -> T",
        ),
    );
    map.insert(
        Text::from("type_max"),
        BuiltinInfo::meta_types(
            meta_type_max,
            "Get maximum value for numeric type",
            "(Type) -> T",
        ),
    );
}

/// Get type size in bytes
fn meta_size_of(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            let size = compute_type_size(&ty.kind)?;
            Ok(ConstValue::Int(size.into()))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get type alignment in bytes
fn meta_align_of(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            let align = compute_type_alignment(&ty.kind)?;
            Ok(ConstValue::Int(align.into()))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get type stride (size with alignment padding for arrays)
fn meta_stride_of(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            let stride = compute_type_stride(&ty.kind)?;
            Ok(ConstValue::Int(stride.into()))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get type size in bits
fn meta_type_bits(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => {
            let size = compute_type_size(&ty.kind)?;
            Ok(ConstValue::Int((size * 8).into()))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get minimum value for numeric types
fn meta_type_min(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => compute_type_min(&ty.kind),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

/// Get maximum value for numeric types
fn meta_type_max(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Type(ty) => compute_type_max(&ty.kind),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Type"),
            found: args[0].type_name(),
        }),
    }
}

// ============================================================================
// Type Property Computation Functions
// ============================================================================

/// Compute the size of a type in bytes
pub fn compute_type_size(ty: &TypeKind) -> Result<u64, MetaError> {
    match ty {
        // Primitives
        TypeKind::Unit | TypeKind::Never => Ok(0),
        TypeKind::Bool => Ok(1),
        TypeKind::Char => Ok(4), // UTF-32 code point
        TypeKind::Int => Ok(8),  // Default 64-bit
        TypeKind::Float => Ok(8), // Default 64-bit
        TypeKind::Text => Ok(24), // ptr + len + capacity
        TypeKind::Unknown => Ok(0), // Unknown has no concrete size

        // References (CBGR: ThinRef = 16 bytes)
        TypeKind::Reference { .. } => Ok(16),
        TypeKind::CheckedReference { .. } => Ok(16),
        TypeKind::UnsafeReference { .. } => Ok(8), // Raw pointer, no CBGR

        // Pointers
        TypeKind::Pointer { .. } => Ok(8),
        TypeKind::VolatilePointer { .. } => Ok(8),

        // Arrays
        TypeKind::Array { element, size } => {
            let elem_stride = compute_type_stride(&element.kind)?;
            // For const expressions, we can't evaluate at this phase
            // Return 0 for dynamic-sized arrays
            if size.is_none() {
                Ok(0)
            } else {
                // Size expression would need evaluation
                // For now, return element stride (single element)
                Ok(elem_stride)
            }
        }

        // Slices (fat pointer: ptr + len)
        TypeKind::Slice(_) => Ok(16),

        // Tuples
        TypeKind::Tuple(elements) => {
            let mut offset = 0u64;
            let mut max_align = 1u64;

            for elem in elements.iter() {
                let align = compute_type_alignment(&elem.kind)?;
                max_align = max_align.max(align);

                // Align offset
                offset = (offset + align - 1) & !(align - 1);
                offset += compute_type_size(&elem.kind)?;
            }

            // Final alignment
            Ok((offset + max_align - 1) & !(max_align - 1))
        }

        // Function types (pointer size)
        TypeKind::Function { .. } | TypeKind::Rank2Function { .. } => Ok(8),

        // Generic types need instantiation
        TypeKind::Generic { base, .. } => compute_type_size(&base.kind),

        // Named types - check for sized primitives first
        TypeKind::Path(path) => {
            let name = path.to_string();
            match name.as_str() {
                // Unsigned integers
                "U8" | "UInt8" | "I8" | "Int8" => Ok(1),
                "U16" | "UInt16" | "I16" | "Int16" => Ok(2),
                "U32" | "UInt32" | "I32" | "Int32" | "F32" | "Float32" => Ok(4),
                "U64" | "UInt64" | "I64" | "Int64" | "F64" | "Float64" => Ok(8),
                "U128" | "UInt128" | "I128" | "Int128" => Ok(16),
                "USize" | "ISize" => Ok(8), // Assuming 64-bit architecture

                // For other named types, we'd need type definition lookup
                _ => Err(MetaError::Other(Text::from(format!(
                    "Cannot compute size of named type '{}' without type definition",
                    name
                )))),
            }
        }

        // Refinement types have same size as base
        TypeKind::Refined { base, .. } => compute_type_size(&base.kind),
        TypeKind::Sigma { base, .. } => compute_type_size(&base.kind),

        // Bounded types have same size as base
        TypeKind::Bounded { base, .. } => compute_type_size(&base.kind),

        // Ownership types
        TypeKind::Ownership { inner, .. } => compute_type_size(&inner.kind),

        // GenRef (generation-aware reference)
        TypeKind::GenRef { .. } => Ok(24), // ptr + generation + epoch

        // Dynamic protocol objects
        TypeKind::DynProtocol { .. } => Ok(16), // vtable ptr + data ptr

        // Other types
        TypeKind::Inferred => Err(MetaError::Other(Text::from(
            "Cannot compute size of inferred type",
        ))),
        TypeKind::Qualified { .. } => Err(MetaError::Other(Text::from(
            "Cannot compute size of qualified type",
        ))),
        TypeKind::TypeConstructor { .. } => Err(MetaError::Other(Text::from(
            "Cannot compute size of type constructor",
        ))),
        TypeKind::Existential { .. } => Err(MetaError::Other(Text::from(
            "Cannot compute size of existential type",
        ))),
        TypeKind::AssociatedType { .. } => Err(MetaError::Other(Text::from(
            "Cannot compute size of associated type",
        ))),
        TypeKind::Tensor { element, .. } => {
            // Tensor size depends on shape, which may be dynamic
            compute_type_size(&element.kind)
        }
        TypeKind::CapabilityRestricted { base, .. } => compute_type_size(&base.kind),
        // Record types: sum of field sizes (with alignment padding)
        TypeKind::Record { fields } => {
            let mut total_size: u64 = 0;
            for field in fields {
                total_size += compute_type_size(&field.ty.kind)?;
            }
            Ok(total_size)
        }
        // Universe types (Type, Type(0), Type(1), ...) are compile-time only
        TypeKind::Universe { .. } => Ok(0),
        // Meta types wrap an inner type
        TypeKind::Meta { inner } => compute_type_size(&inner.kind),
        // Type lambdas are compile-time only
        TypeKind::TypeLambda { .. } => Ok(0),
        // PathType is a dependent propositional equality type — compile-time only (size 0)
        TypeKind::PathType { .. } => Ok(0),
    }
}

/// Compute the alignment of a type in bytes
pub fn compute_type_alignment(ty: &TypeKind) -> Result<u64, MetaError> {
    match ty {
        // Primitives
        TypeKind::Unit | TypeKind::Never | TypeKind::Unknown => Ok(1),
        TypeKind::Bool => Ok(1),
        TypeKind::Char => Ok(4),
        TypeKind::Int => Ok(8),
        TypeKind::Float => Ok(8),
        TypeKind::Text => Ok(8),

        // References and pointers
        TypeKind::Reference { .. }
        | TypeKind::CheckedReference { .. }
        | TypeKind::UnsafeReference { .. }
        | TypeKind::Pointer { .. }
        | TypeKind::VolatilePointer { .. }
        | TypeKind::Slice(_) => Ok(8),

        // Arrays
        TypeKind::Array { element, .. } => compute_type_alignment(&element.kind),

        // Tuples (max alignment of elements)
        TypeKind::Tuple(elements) => {
            let mut max_align = 1u64;
            for elem in elements.iter() {
                max_align = max_align.max(compute_type_alignment(&elem.kind)?);
            }
            Ok(max_align)
        }

        // Function types
        TypeKind::Function { .. } | TypeKind::Rank2Function { .. } => Ok(8),

        // Generic types
        TypeKind::Generic { base, .. } => compute_type_alignment(&base.kind),

        // Named types - check for sized primitives first
        TypeKind::Path(path) => {
            let name = path.to_string();
            match name.as_str() {
                // Single-byte types align to 1
                "U8" | "UInt8" | "I8" | "Int8" => Ok(1),
                // Two-byte types align to 2
                "U16" | "UInt16" | "I16" | "Int16" => Ok(2),
                // Four-byte types align to 4
                "U32" | "UInt32" | "I32" | "Int32" | "F32" | "Float32" => Ok(4),
                // Eight-byte types align to 8
                "U64" | "UInt64" | "I64" | "Int64" | "F64" | "Float64" |
                "USize" | "ISize" => Ok(8),
                // 16-byte types align to 16
                "U128" | "UInt128" | "I128" | "Int128" => Ok(16),

                // For other named types, we'd need type definition lookup
                _ => Err(MetaError::Other(Text::from(format!(
                    "Cannot compute alignment of named type '{}' without type definition",
                    name
                )))),
            }
        },

        // Refinement types
        TypeKind::Refined { base, .. } => compute_type_alignment(&base.kind),
        TypeKind::Sigma { base, .. } => compute_type_alignment(&base.kind),

        // Bounded types
        TypeKind::Bounded { base, .. } => compute_type_alignment(&base.kind),

        // Ownership types
        TypeKind::Ownership { inner, .. } => compute_type_alignment(&inner.kind),

        // GenRef
        TypeKind::GenRef { .. } => Ok(8),

        // Dynamic protocol objects
        TypeKind::DynProtocol { .. } => Ok(8),

        // Tensor
        TypeKind::Tensor { element, .. } => compute_type_alignment(&element.kind),

        // Capability-restricted
        TypeKind::CapabilityRestricted { base, .. } => compute_type_alignment(&base.kind),

        // Record types: max alignment of all field types
        TypeKind::Record { fields } => {
            let mut max_align: u64 = 1;
            for field in fields {
                max_align = max_align.max(compute_type_alignment(&field.ty.kind)?);
            }
            Ok(max_align)
        }

        // Universe types (compile-time only)
        TypeKind::Universe { .. } => Ok(1),

        // Other types
        TypeKind::Inferred
        | TypeKind::Qualified { .. }
        | TypeKind::TypeConstructor { .. }
        | TypeKind::Existential { .. }
        | TypeKind::AssociatedType { .. } => Err(MetaError::Other(Text::from(
            "Cannot compute alignment of abstract type",
        ))),

        // Meta types wrap an inner type
        TypeKind::Meta { inner } => compute_type_alignment(&inner.kind),
        // Type lambdas are compile-time only
        TypeKind::TypeLambda { .. } => Ok(1),
        // PathType is a dependent propositional equality type — compile-time only (align 1)
        TypeKind::PathType { .. } => Ok(1),
    }
}

/// Compute the stride of a type (size rounded up to alignment)
pub fn compute_type_stride(ty: &TypeKind) -> Result<u64, MetaError> {
    let size = compute_type_size(ty)?;
    let align = compute_type_alignment(ty)?;

    // Round size up to alignment
    Ok((size + align - 1) & !(align - 1))
}

/// Compute the minimum value for numeric types
pub fn compute_type_min(ty: &TypeKind) -> Result<ConstValue, MetaError> {
    match ty {
        // Default Int is 64-bit signed
        TypeKind::Int => Ok(ConstValue::Int(i64::MIN as i128)),
        // Float has negative infinity conceptually, but we use MIN
        TypeKind::Float => Ok(ConstValue::Float(f64::MIN)),
        TypeKind::Bool => Ok(ConstValue::Bool(false)),
        TypeKind::Char => Ok(ConstValue::Char('\0')),

        // Sized integer types (via Path)
        TypeKind::Path(path) => {
            let name = path.to_string();
            match name.as_str() {
                // Unsigned integers - min is always 0
                "U8" | "UInt8" => Ok(ConstValue::Int(0)),
                "U16" | "UInt16" => Ok(ConstValue::Int(0)),
                "U32" | "UInt32" => Ok(ConstValue::Int(0)),
                "U64" | "UInt64" => Ok(ConstValue::Int(0)),
                "U128" | "UInt128" => Ok(ConstValue::Int(0)),
                "USize" => Ok(ConstValue::Int(0)),

                // Signed integers - min is -(2^(bits-1))
                "I8" | "Int8" => Ok(ConstValue::Int(i8::MIN as i128)),
                "I16" | "Int16" => Ok(ConstValue::Int(i16::MIN as i128)),
                "I32" | "Int32" => Ok(ConstValue::Int(i32::MIN as i128)),
                "I64" | "Int64" => Ok(ConstValue::Int(i64::MIN as i128)),
                "I128" | "Int128" => Ok(ConstValue::Int(i128::MIN)),
                "ISize" => Ok(ConstValue::Int(isize::MIN as i128)),

                // Float types
                "F32" | "Float32" => Ok(ConstValue::Float(f32::MIN as f64)),
                "F64" | "Float64" => Ok(ConstValue::Float(f64::MIN)),

                _ => Err(MetaError::Other(Text::from(format!(
                    "Type {} does not have a minimum value",
                    name
                )))),
            }
        }

        // Refined types delegate to base
        TypeKind::Refined { base, .. } => compute_type_min(&base.kind),
        TypeKind::Sigma { base, .. } => compute_type_min(&base.kind),
        TypeKind::Bounded { base, .. } => compute_type_min(&base.kind),
        TypeKind::CapabilityRestricted { base, .. } => compute_type_min(&base.kind),

        _ => Err(MetaError::Other(Text::from(format!(
            "Type {:?} does not have a minimum value",
            ty
        )))),
    }
}

/// Compute the maximum value for numeric types
pub fn compute_type_max(ty: &TypeKind) -> Result<ConstValue, MetaError> {
    match ty {
        // Default Int is 64-bit signed
        TypeKind::Int => Ok(ConstValue::Int(i64::MAX as i128)),
        // Float has positive infinity conceptually, but we use MAX
        TypeKind::Float => Ok(ConstValue::Float(f64::MAX)),
        TypeKind::Bool => Ok(ConstValue::Bool(true)),
        TypeKind::Char => Ok(ConstValue::Char(char::MAX)),

        // Sized integer types (via Path)
        TypeKind::Path(path) => {
            let name = path.to_string();
            match name.as_str() {
                // Unsigned integers - max is 2^bits - 1
                "U8" | "UInt8" => Ok(ConstValue::Int(u8::MAX as i128)),
                "U16" | "UInt16" => Ok(ConstValue::Int(u16::MAX as i128)),
                "U32" | "UInt32" => Ok(ConstValue::Int(u32::MAX as i128)),
                "U64" | "UInt64" => Ok(ConstValue::Int(u64::MAX as i128)),
                "U128" | "UInt128" => Ok(ConstValue::Int(u128::MAX as i128)),
                "USize" => Ok(ConstValue::Int(usize::MAX as i128)),

                // Signed integers - max is 2^(bits-1) - 1
                "I8" | "Int8" => Ok(ConstValue::Int(i8::MAX as i128)),
                "I16" | "Int16" => Ok(ConstValue::Int(i16::MAX as i128)),
                "I32" | "Int32" => Ok(ConstValue::Int(i32::MAX as i128)),
                "I64" | "Int64" => Ok(ConstValue::Int(i64::MAX as i128)),
                "I128" | "Int128" => Ok(ConstValue::Int(i128::MAX)),
                "ISize" => Ok(ConstValue::Int(isize::MAX as i128)),

                // Float types
                "F32" | "Float32" => Ok(ConstValue::Float(f32::MAX as f64)),
                "F64" | "Float64" => Ok(ConstValue::Float(f64::MAX)),

                _ => Err(MetaError::Other(Text::from(format!(
                    "Type {} does not have a maximum value",
                    name
                )))),
            }
        }

        // Refined types delegate to base
        TypeKind::Refined { base, .. } => compute_type_max(&base.kind),
        TypeKind::Sigma { base, .. } => compute_type_max(&base.kind),
        TypeKind::Bounded { base, .. } => compute_type_max(&base.kind),
        TypeKind::CapabilityRestricted { base, .. } => compute_type_max(&base.kind),

        _ => Err(MetaError::Other(Text::from(format!(
            "Type {:?} does not have a maximum value",
            ty
        )))),
    }
}

/// Get the name of a type as Text
pub fn compute_type_name(ty: &TypeKind) -> Text {
    match ty {
        TypeKind::Unit => Text::from("()"),
        TypeKind::Never => Text::from("!"),
        TypeKind::Unknown => Text::from("unknown"),
        TypeKind::Bool => Text::from("Bool"),
        TypeKind::Int => Text::from("Int"),
        TypeKind::Float => Text::from("Float"),
        TypeKind::Char => Text::from("Char"),
        TypeKind::Text => Text::from("Text"),

        TypeKind::Path(path) => Text::from(path.to_string()),

        TypeKind::Tuple(elements) => {
            let names: Vec<String> = elements
                .iter()
                .map(|e| compute_type_name(&e.kind).to_string())
                .collect();
            Text::from(format!("({})", names.join(", ")))
        }

        TypeKind::Array { element, size } => {
            let elem_name = compute_type_name(&element.kind);
            if size.is_some() {
                Text::from(format!("[{}; _]", elem_name))
            } else {
                Text::from(format!("[{}]", elem_name))
            }
        }

        TypeKind::Slice(inner) => {
            Text::from(format!("[{}]", compute_type_name(&inner.kind)))
        }

        TypeKind::Reference { mutable, inner } => {
            if *mutable {
                Text::from(format!("&mut {}", compute_type_name(&inner.kind)))
            } else {
                Text::from(format!("&{}", compute_type_name(&inner.kind)))
            }
        }

        TypeKind::CheckedReference { mutable, inner } => {
            if *mutable {
                Text::from(format!("&checked mut {}", compute_type_name(&inner.kind)))
            } else {
                Text::from(format!("&checked {}", compute_type_name(&inner.kind)))
            }
        }

        TypeKind::UnsafeReference { mutable, inner } => {
            if *mutable {
                Text::from(format!("&unsafe mut {}", compute_type_name(&inner.kind)))
            } else {
                Text::from(format!("&unsafe {}", compute_type_name(&inner.kind)))
            }
        }

        TypeKind::Pointer { mutable, inner } => {
            if *mutable {
                Text::from(format!("*mut {}", compute_type_name(&inner.kind)))
            } else {
                Text::from(format!("*const {}", compute_type_name(&inner.kind)))
            }
        }

        TypeKind::VolatilePointer { mutable, inner } => {
            if *mutable {
                Text::from(format!("*volatile mut {}", compute_type_name(&inner.kind)))
            } else {
                Text::from(format!("*volatile {}", compute_type_name(&inner.kind)))
            }
        }

        TypeKind::Function { params, return_type, .. } => {
            let param_names: Vec<String> = params
                .iter()
                .map(|p| compute_type_name(&p.kind).to_string())
                .collect();
            let ret_name = compute_type_name(&return_type.kind);
            Text::from(format!("fn({}) -> {}", param_names.join(", "), ret_name))
        }

        TypeKind::Generic { base, args } => {
            let base_name = compute_type_name(&base.kind);
            let arg_names: Vec<String> = args
                .iter()
                .map(|arg| match arg {
                    verum_ast::ty::GenericArg::Type(t) => compute_type_name(&t.kind).to_string(),
                    verum_ast::ty::GenericArg::Const(_) => "_".to_string(),
                    verum_ast::ty::GenericArg::Lifetime(lt) => format!("'{}", lt.name),
                    verum_ast::ty::GenericArg::Binding(b) => {
                        format!("{} = {}", b.name.name, compute_type_name(&b.ty.kind))
                    }
                })
                .collect();
            Text::from(format!("{}<{}>", base_name, arg_names.join(", ")))
        }

        TypeKind::Refined { base, .. } => {
            Text::from(format!("{}{{...}}", compute_type_name(&base.kind)))
        }

        TypeKind::Inferred => Text::from("_"),

        _ => Text::from(format!("{:?}", ty)),
    }
}

/// Compute a unique type ID based on type structure using Blake3.
pub fn compute_type_id(ty: &TypeKind) -> u64 {
    let mut hasher = crate::hash::ContentHash::new();
    let name = compute_type_name(ty);
    hasher.update_str(name.as_str());
    hasher.finalize().to_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primitive_sizes() {
        assert_eq!(compute_type_size(&TypeKind::Bool).unwrap(), 1);
        assert_eq!(compute_type_size(&TypeKind::Int).unwrap(), 8);
        assert_eq!(compute_type_size(&TypeKind::Float).unwrap(), 8);
        assert_eq!(compute_type_size(&TypeKind::Char).unwrap(), 4);
    }

    #[test]
    fn test_primitive_alignments() {
        assert_eq!(compute_type_alignment(&TypeKind::Bool).unwrap(), 1);
        assert_eq!(compute_type_alignment(&TypeKind::Int).unwrap(), 8);
        assert_eq!(compute_type_alignment(&TypeKind::Float).unwrap(), 8);
    }

    #[test]
    fn test_stride_calculation() {
        // Stride should be size rounded up to alignment
        assert_eq!(compute_type_stride(&TypeKind::Int).unwrap(), 8);
        assert_eq!(compute_type_stride(&TypeKind::Bool).unwrap(), 1);
    }

    #[test]
    fn test_type_min_max() {
        assert_eq!(
            compute_type_min(&TypeKind::Int).unwrap(),
            ConstValue::Int(i64::MIN as i128)
        );
        assert_eq!(
            compute_type_max(&TypeKind::Int).unwrap(),
            ConstValue::Int(i64::MAX as i128)
        );
    }

    #[test]
    fn test_type_name() {
        assert_eq!(compute_type_name(&TypeKind::Int), Text::from("Int"));
        assert_eq!(compute_type_name(&TypeKind::Bool), Text::from("Bool"));
        assert_eq!(compute_type_name(&TypeKind::Unit), Text::from("()"));
    }
}
