//! Resolve only declaration-carried associated assignments.
//!
//! Applied families require a declaration-local telescope. Until that evidence
//! is present, consumers retain the full projection instead of guessing from a
//! method name or replacing the projection with its receiver.
use crate::{VbcError, VbcResult, module::VbcModule, mono::TypeSubstitution,
    types::{ProtocolApplication, TypeRef}};

/// Resolve a plain associated assignment from the receiver's exact descriptor.
/// Missing or ambiguous evidence remains unresolved. Invalid carriers fail.
pub fn resolve_projection(
    module: &VbcModule,
    base: &TypeRef,
    qualifier: Option<&ProtocolApplication>,
    assoc: &str,
    arguments: &[TypeRef],
) -> VbcResult<Option<TypeRef>> {
    // The legacy assignment map has no local family binders. Even zero supplied
    // arguments can mean a defaulted family, so a qualified application also
    // waits for its complete declaration-owned assignment carrier.
    if qualifier.is_some() || !arguments.is_empty() { return Ok(None); }
    let (id, arguments) = match base {
        TypeRef::Concrete(id) => (*id, &[][..]),
        TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
        _ => return Ok(None),
    };
    let Some(owner) = module.get_type(id) else { return Ok(None); };
    if owner.type_params.len() != arguments.len() {
        return Err(VbcError::AssociatedType("associated receiver argument count differs from its declaration".into()));
    }
    let substitution = TypeSubstitution::new(&owner.type_params, arguments);
    let mut result = None;
    for implementation in &owner.protocols {
        for (name, body) in &implementation.associated_types {
            if module.get_string(*name) != Some(assoc) { continue; }
            let Some(protocol) = module.get_type(crate::types::TypeId(implementation.protocol.0)) else { return Ok(None); };
            let Some(source) = &protocol.source_declaration else { return Ok(None); };
            let declaration = match &source.body {
                verum_ast::TypeDeclBody::Protocol(body) | verum_ast::TypeDeclBody::Coinductive(body) => body,
                _ => return Ok(None),
            };
            let Some(parameters) = declaration.items.iter().find_map(|item| match &item.kind {
                verum_ast::decl::ProtocolItemKind::Type { name, type_params, .. } if name.name.as_str() == assoc => Some(type_params),
                _ => None,
            }) else { return Ok(None); };
            if !parameters.is_empty() { return Ok(None); }
            if result.is_some() { return Ok(None); }
            result = Some(substitution.apply(body)
                .map_err(|error| VbcError::AssociatedType(error.to_string()))?);
        }
    }
    Ok(result)
}

/// Instantiate the declaration's binders before reducing their associated members.
/// VM frame arguments and native specializations share this exact boundary.
pub(crate) fn instantiate_witness(
    module: &VbcModule,
    substitution: &TypeSubstitution,
    ty: &TypeRef,
) -> VbcResult<TypeRef> {
    let mut result = if substitution.is_empty() { ty.clone() } else {
        substitution.apply(ty).map_err(|error| VbcError::AssociatedType(error.to_string()))?
    };
    normalize_projections(module, &mut result)?;
    Ok(result)
}

/// Normalize declaration-backed associated assignments throughout a witness.
/// An unresolved member stays explicit for the consumer that requires it.
/// Bound both depth and expansion work, including branching recursive families.
pub(crate) fn normalize_projections(module: &VbcModule, ty: &mut TypeRef) -> VbcResult<()> {
    fn normalize(
        module: &VbcModule,
        ty: &mut TypeRef,
        depth: usize,
        remaining: &mut usize,
    ) -> VbcResult<()> {
        if depth > crate::mono::MAX_TYPE_REF_DEPTH || *remaining == 0 {
            return Err(VbcError::AssociatedType("associated-type expansion exceeds the type nesting or work limit".into()));
        }
        *remaining -= 1;
        let mut child = |ty: &mut TypeRef| normalize(module, ty, depth + 1, remaining);
        match ty {
            TypeRef::AssociatedProjection { base, qualifier, assoc, args } => {
                child(base)?;
                if let Some(qualifier) = qualifier {
                    qualifier.args.iter_mut().try_for_each(&mut child)?;
                }
                args.iter_mut().try_for_each(&mut child)?;
                if let Some(result) = resolve_projection(module, base, qualifier.as_ref(), assoc, args)? {
                    if result == *ty {
                        return Err(VbcError::AssociatedType("cyclic associated-type binding".into()));
                    }
                    *ty = result;
                    child(ty)?;
                }
            }
            TypeRef::Dynamic(object) => object.types_mut().try_for_each(child)?,
            TypeRef::Instantiated { args, .. } | TypeRef::Tuple(args) => {
                args.iter_mut().try_for_each(child)?;
            }
            TypeRef::Function { params, return_type, .. } | TypeRef::Rank2Function { params, return_type, .. } => {
                params.iter_mut().try_for_each(&mut child)?;
                child(return_type)?;
            }
            TypeRef::Reference { inner, .. } | TypeRef::Slice(inner) | TypeRef::Array { element: inner, .. } => child(inner)?,
            TypeRef::Concrete(_) | TypeRef::Generic(_) | TypeRef::ConstValue(_) | TypeRef::ConstSequence(_)
                | TypeRef::RecordFieldProjection { .. } => {}
        }
        Ok(())
    }
    const MAX_EXPANSION_NODES: usize = 16_384;
    let mut remaining = MAX_EXPANSION_NODES;
    normalize(module, ty, 0, &mut remaining)
}
