//! Nominal error payloads of the fallible allocation operations.
use crate::module::VbcModule;
use crate::types::{TypeId, TypeKind, TypeRef, VariantKind};
use verum_common::{Text, well_known_types::WellKnownType};

/// Semantic failure reported by allocation and reallocation.
#[derive(Clone, Copy, Debug)]
pub enum AllocationFailure {
    /// The arguments violate the allocation contract.
    InvalidSize,
    /// The requested storage could not be acquired.
    OutOfMemory,
}

/// The declaration that owns the operation's Result error type.
#[derive(Clone, Copy, Debug)]
pub enum AllocatingOperation<'a> {
    /// Registered intrinsic identity.
    Intrinsic(&'a str),
    /// Operation on the standard Map declaration.
    MapMethod(&'a str),
}

/// Resolve E and its discriminant from the selected operation's Result<_, E>.
pub fn error_variant(
    module: &VbcModule,
    operation: AllocatingOperation<'_>,
    error: AllocationFailure,
) -> Result<(TypeId, u32), Text> {
    let (variant, field) = match error {
        AllocationFailure::InvalidSize => ("InvalidSize", "size"),
        AllocationFailure::OutOfMemory => ("OutOfMemory", "requested"),
    };
    let mut owners = module.functions.iter().filter_map(|function| {
        match operation {
            AllocatingOperation::Intrinsic(name) => {
                if function.intrinsic_name.and_then(|id| module.get_string(id)) != Some(name) {
                    return None;
                }
            }
            AllocatingOperation::MapMethod(method) => {
                if module.get_string(function.name)?.rsplit('.').next()? != method {
                    return None;
                }
                let parent = module.get_type(function.parent_type?)?;
                let owner = parent
                    .origin_module
                    .and_then(|id| module.get_string(id))
                    .unwrap_or(&module.name);
                if WellKnownType::from_declaration(module.get_string(parent.name)?, owner)
                    != Some(WellKnownType::Map)
                {
                    return None;
                }
            }
        }
        let TypeRef::Instantiated {
            base: TypeId::RESULT,
            args,
        } = &function.return_type
        else {
            return None;
        };
        let [_, TypeRef::Concrete(owner)] = args.as_slice() else {
            return None;
        };
        Some(*owner)
    });
    let invalid = || {
        Text::from(format!(
            "{operation:?} has no unique Result error declaring {variant} {{ {field}: Int }}"
        ))
    };
    let owner = owners.next().ok_or_else(invalid)?;
    if owners.any(|id| id != owner) {
        return Err(invalid());
    }
    let declaration = module
        .get_type(owner)
        .filter(|ty| ty.kind == TypeKind::Sum)
        .ok_or_else(invalid)?;
    let mut cases = declaration
        .variants
        .iter()
        .filter(|case| module.get_string(case.name) == Some(variant));
    let case = cases.next().ok_or_else(invalid)?;
    if cases.next().is_some()
        || declaration
            .variants
            .iter()
            .filter(|other| other.tag == case.tag)
            .count()
            != 1
    {
        return Err(invalid());
    }
    let [payload] = case.fields.as_slice() else {
        return Err(invalid());
    };
    if case.kind != VariantKind::Record
        || module.get_string(payload.name) != Some(field)
        || payload.type_ref != TypeRef::Concrete(TypeId::INT)
    {
        return Err(invalid());
    }
    Ok((owner, case.tag))
}
