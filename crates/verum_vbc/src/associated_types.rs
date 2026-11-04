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
        return Err(VbcError::Deserialization("associated receiver argument count differs from its declaration".into()));
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
                .map_err(|error| VbcError::Deserialization(error.to_string()))?);
        }
    }
    Ok(result)
}
