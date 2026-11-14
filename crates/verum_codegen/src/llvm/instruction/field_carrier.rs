//! Declaration-owned representation used by native indexed field reads.
use super::super::error::{LlvmLoweringError, Result};
use verum_common::{List, Set};
use verum_vbc::source_declaration_integrity::{transparent_value_type, validate_type_descriptor};
use verum_vbc::types::{MAX_TYPE_REF_NODES, TypeDescriptor, TypeId, TypeKind, TypeRef};
use verum_vbc::{VbcModule, mono::TypeSubstitution};

/// A transparent declaration adds no runtime field. Resolve its actual payload
/// before deciding whether GetF reads a record or preserves a scalar. This only
/// selects a layout; reference access remains the caller's responsibility.
pub(super) fn transparent_carrier(
    module: &VbcModule,
    declaration: &TypeDescriptor,
    args: &[TypeRef],
) -> Result<Option<TypeRef>> {
    let Some(payload) = transparent_value_type(declaration, &module.strings)
        .map_err(LlvmLoweringError::InvalidType)?
    else {
        return Ok(None);
    };
    let mut current = substitute(declaration, args, payload)?;
    let mut seen = Set::new();
    for _ in 0..MAX_TYPE_REF_NODES {
        if !seen.insert(current.clone()) {
            return Err(LlvmLoweringError::InvalidType(
                "cyclic transparent field carrier".into(),
            ));
        }
        let (owner, args) = match &current {
            TypeRef::Concrete(owner) => (*owner, &[][..]),
            TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
            _ => return Ok(Some(current)),
        };
        let Some(declaration) = module.get_type(owner) else {
            if owner.is_builtin() {
                return Ok(Some(current));
            }
            return Err(LlvmLoweringError::InvalidType(
                "transparent field carrier has no declared payload owner".into(),
            ));
        };
        let next = transparent_value_type(declaration, &module.strings)
            .map_err(LlvmLoweringError::InvalidType)?;
        let next = if let Some(payload) = next {
            payload
        } else if declaration.kind == TypeKind::Alias {
            validate_type_descriptor(declaration, &module.strings)
                .map_err(LlvmLoweringError::InvalidType)?;
            declaration.alias_target.as_ref().ok_or_else(|| {
                LlvmLoweringError::InvalidType("field carrier alias has no target".into())
            })?
        } else {
            return Ok(Some(current));
        };
        current = substitute(declaration, args, next)?;
    }
    Err(LlvmLoweringError::InvalidType(
        "transparent field carrier exceeds type work budget".into(),
    ))
}

fn substitute(declaration: &TypeDescriptor, args: &[TypeRef], value: &TypeRef) -> Result<TypeRef> {
    if declaration.type_params.len() != args.len() {
        return Err(LlvmLoweringError::InvalidType(
            "transparent field carrier has incomplete owner arguments".into(),
        ));
    }
    let value = TypeSubstitution::new(&declaration.type_params, args)
        .apply(value)
        .map_err(|error| LlvmLoweringError::InvalidType(error.to_string().into()))?;
    if value.is_generic() {
        return Err(LlvmLoweringError::InvalidType(
            "transparent field carrier has unresolved owner arguments".into(),
        ));
    }
    Ok(value)
}

/// Separate the layout owner from reference qualifiers. No new reference or
/// access permission is produced by this metadata query.
pub(super) fn nominal_parts(ty: &TypeRef) -> Option<(TypeId, List<TypeRef>)> {
    let mut current = ty;
    for _ in 0..MAX_TYPE_REF_NODES {
        match current {
            TypeRef::Reference { inner, .. } => current = inner,
            TypeRef::Concrete(owner) => return Some((*owner, List::new())),
            TypeRef::Instantiated { base, args } => return Some((*base, args.clone().into())),
            _ => return None,
        }
    }
    None
}
