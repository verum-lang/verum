//! Declaration layout queries, separate from object allocation and List storage.
//!
//! Nominal layouts come from the declaring producer. Missing archive facts stay
//! unknown; an object's slot extent is never substituted for semantic layout.

use crate::instruction::LayoutProperty;
use crate::module::VbcModule;
use crate::types::{CbgrTier, TypeDescriptor, TypeId, TypeKind, TypeRef};
use verum_common::layout as abi;

/// Resolve a layout query using a module's exact nominal identities.
pub fn query(module: &VbcModule, ty: &TypeRef, property: LayoutProperty) -> Option<u64> {
    query_with(ty, property, |id| module.get_type(id))
}

/// Shared producer/runtime authority; the lookup must use exact TypeIds.
pub fn query_with<'a>(
    ty: &TypeRef,
    property: LayoutProperty,
    lookup: impl Fn(TypeId) -> Option<&'a TypeDescriptor>,
) -> Option<u64> {
    if property == LayoutProperty::IsSigned {
        return signedness(ty, &lookup, 0).map(u64::from);
    }
    let (size, alignment) = layout(ty, &lookup, 0)?;
    if alignment == 0 || !alignment.is_power_of_two() {
        return None;
    }
    let result = match property {
        LayoutProperty::Size => Some(size),
        LayoutProperty::Bits => size.checked_mul(8),
        LayoutProperty::Alignment => Some(alignment),
        LayoutProperty::Stride => size
            .checked_add(alignment - 1)
            .map(|n| n & !(alignment - 1)),
        LayoutProperty::IsSigned => unreachable!("handled before layout"),
    };
    // Source properties return Int; never wrap an extent into a negative count.
    result.filter(|value| i64::try_from(*value).is_ok())
}

fn alias_target(ty: &TypeRef, descriptor: &TypeDescriptor) -> Option<TypeRef> {
    let args = match ty {
        TypeRef::Instantiated { args, .. } => args.as_slice(),
        _ => &[],
    };
    if args.len() != descriptor.type_params.len() {
        return None;
    }
    crate::mono::TypeSubstitution::new(&descriptor.type_params, args)
        .apply(descriptor.alias_target.as_ref()?).ok()
}

fn signedness<'a>(
    ty: &TypeRef,
    lookup: &impl Fn(TypeId) -> Option<&'a TypeDescriptor>,
    depth: usize,
) -> Option<bool> {
    if depth >= 64 { return None; }
    match ty {
        TypeRef::Concrete(id) | TypeRef::Instantiated { base: id, .. } => {
            if let Some(name) = id.well_known_name() {
                return Some(id.is_float() || verum_common::well_known_types::type_names::is_signed_integer_type(name));
            }
            let descriptor = lookup(*id)?;
            if descriptor.kind == TypeKind::Alias {
                signedness(&alias_target(ty, descriptor)?, lookup, depth + 1)
            } else {
                Some(false)
            }
        }
        TypeRef::Generic(_) | TypeRef::AssociatedProjection { .. } | TypeRef::RecordFieldProjection { .. }
        | TypeRef::ConstValue(_) | TypeRef::ConstSequence(_) | TypeRef::Dynamic(_) => None,
        TypeRef::Array { .. } | TypeRef::Slice(_) | TypeRef::Reference { .. } | TypeRef::Tuple(_)
        | TypeRef::Function { .. } | TypeRef::Rank2Function { .. } => Some(false),
    }
}

fn layout<'a>(
    ty: &TypeRef,
    lookup: &impl Fn(TypeId) -> Option<&'a TypeDescriptor>,
    depth: usize,
) -> Option<(u64, u64)> {
    if depth >= 64 {
        return None;
    }
    match ty {
        TypeRef::Concrete(id) | TypeRef::Instantiated { base: id, .. } => {
            if *id == TypeId::PTR {
                return Some((abi::POINTER_SIZE, abi::POINTER_SIZE));
            }
            if let Some(name) = id.well_known_name()
                && let Some(size) = abi::primitive_size_by_name(name)
            {
                return Some((size, abi::primitive_alignment_by_name(name)?));
            }
            let descriptor = lookup(*id)?;
            if descriptor.kind == TypeKind::Alias {
                return layout(&alias_target(ty, descriptor)?, lookup, depth + 1);
            }
            descriptor
                .declared_layout
                .map(|fact| (fact.size, fact.alignment))
        }
        TypeRef::Array { element, length } => {
            let (size, alignment) = layout(element, lookup, depth + 1)?;
            if alignment == 0 || !alignment.is_power_of_two() {
                return None;
            }
            let stride = size.checked_add(alignment - 1)? & !(alignment - 1);
            Some((stride.checked_mul(*length)?, alignment))
        }
        TypeRef::Reference { inner, tier, .. } => {
            let fat = unsized_pointee(inner, lookup, depth + 1)?;
            let size = match (tier, fat) {
                (CbgrTier::Tier2, false) => abi::POINTER_SIZE,
                (CbgrTier::Tier2, true) => abi::SLICE_FAT_PTR_SIZE,
                (_, false) => abi::THIN_REF_SIZE,
                (_, true) => abi::FAT_REF_SIZE,
            };
            Some((size, abi::POINTER_SIZE))
        }
        TypeRef::Tuple(elements) => Some((
            (elements.len() as u64).checked_mul(abi::VALUE_SLOT_SIZE)?,
            abi::VALUE_SLOT_SIZE,
        )),
        TypeRef::Function { .. } | TypeRef::Rank2Function { .. } => {
            Some((abi::POINTER_SIZE, abi::POINTER_SIZE))
        }
        TypeRef::Generic(_)
        | TypeRef::Slice(_)
        | TypeRef::AssociatedProjection { .. }
        | TypeRef::RecordFieldProjection { .. }
        | TypeRef::ConstValue(_) | TypeRef::ConstSequence(_) | TypeRef::Dynamic(_) => None,
    }
}

// Pointee classification follows alias declarations too: `&Alias<[T]>` is
// still fat. Unknown declarations/generics cannot silently become thin.
fn unsized_pointee<'a>(
    ty: &TypeRef,
    lookup: &impl Fn(TypeId) -> Option<&'a TypeDescriptor>,
    depth: usize,
) -> Option<bool> {
    if depth >= 64 {
        return None;
    }
    match ty {
        TypeRef::Slice(_) | TypeRef::Dynamic(_) => Some(true),
        TypeRef::Generic(_) | TypeRef::AssociatedProjection { .. }
        | TypeRef::RecordFieldProjection { .. } => None,
        TypeRef::Concrete(id) | TypeRef::Instantiated { base: id, .. } => {
            if id.well_known_name().is_some() || *id == TypeId::PTR {
                return Some(false);
            }
            let descriptor = lookup(*id)?;
            if descriptor.kind == TypeKind::Alias {
                return unsized_pointee(&alias_target(ty, descriptor)?, lookup, depth + 1);
            }
            Some(descriptor.kind == TypeKind::Protocol)
        }
        _ => Some(false),
    }
}
