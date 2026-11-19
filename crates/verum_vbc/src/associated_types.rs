//! Resolve only declaration-carried associated assignments.
//!
//! Applied families require a declaration-local telescope. Until that evidence
//! is present, consumers retain the full projection instead of guessing from a
//! method name or replacing the projection with its receiver.
use crate::{VbcError, VbcResult, module::VbcModule, mono::TypeSubstitution,
    types::{ProtocolApplication, StringId, TypeDescriptor, TypeId, TypeRef}};

/// Declaration and name IDs belong to one table. Source production and the
/// serialized module use different name pools but interpret equations alike.
pub(crate) trait DeclarationLookup {
    fn descriptor(&self, id: TypeId) -> Option<&TypeDescriptor>;
    fn spelling(&self, id: StringId) -> Option<&str>;
}

impl DeclarationLookup for VbcModule {
    fn descriptor(&self, id: TypeId) -> Option<&TypeDescriptor> { self.get_type(id) }
    fn spelling(&self, id: StringId) -> Option<&str> { self.get_string(id) }
}

/// Resolve a plain associated assignment from the receiver's exact descriptor.
/// Missing or ambiguous evidence remains unresolved. Invalid carriers fail.
pub fn resolve_projection(
    module: &VbcModule,
    base: &TypeRef,
    qualifier: Option<&ProtocolApplication>,
    assoc: &str,
    arguments: &[TypeRef],
) -> VbcResult<Option<TypeRef>> {
    projection_in(module, base, qualifier, assoc, arguments)
}

fn projection_in(
    declarations: &impl DeclarationLookup,
    base: &TypeRef,
    qualifier: Option<&ProtocolApplication>,
    assoc: &str,
    arguments: &[TypeRef],
) -> VbcResult<Option<TypeRef>> {
    if !arguments.is_empty() { return Ok(None); }
    let (id, arguments) = match base {
        TypeRef::Concrete(id) => (*id, &[][..]),
        TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
        _ => return Ok(None),
    };
    let Some(owner) = declarations.descriptor(id) else { return Ok(None); };
    if owner.type_params.len() != arguments.len() {
        return Err(VbcError::AssociatedType("associated receiver argument count differs from its declaration".into()));
    }
    if qualifier.is_some_and(|application| !application.args.is_empty()) { return Ok(None); }
    let mut selected: Option<&crate::types::ProtocolImpl> = None;
    for row in &owner.protocols {
        if qualifier.is_some_and(|application| application.protocol != row.protocol) { continue; }
        let Some(protocol) = declarations.descriptor(TypeId(row.protocol.0)) else { return Ok(None); };
        let Some(source) = &protocol.source_declaration else { return Ok(None); };
        let declaration = match &source.body {
            verum_ast::TypeDeclBody::Protocol(body) | verum_ast::TypeDeclBody::Coinductive(body) => body,
            _ => return Ok(None),
        };
        let member = declaration.items.iter().find_map(|item| match &item.kind {
            verum_ast::decl::ProtocolItemKind::Type { name, type_params, .. } if name.name.as_str() == assoc => Some(type_params),
            _ => None,
        });
        let Some(parameters) = member else { continue; };
        // An erased equation is still a declaration of this member. It cannot
        // make a competing protocol the unique owner of an unqualified name.
        if !protocol.type_params.is_empty() || !parameters.is_empty()
            || !row.source_implementation.as_ref().is_some_and(|source| source.has_unconditional_member(assoc))
        { return Ok(None); }
        if let Some(previous) = selected {
            if row.protocol != previous.protocol || row.source_implementation != previous.source_implementation
                || row.associated_types != previous.associated_types
            { return Ok(None); }
        } else {
            selected = Some(row);
        }
    }
    let Some(row) = selected else { return Ok(None); };
    let mut equations = row.associated_types.iter().filter(|(name, _)| declarations.spelling(*name) == Some(assoc));
    let Some((_, body)) = equations.next() else { return Ok(None); };
    if equations.next().is_some() { return Ok(None); }
    Ok(Some(TypeSubstitution::new(&owner.type_params, arguments).apply(body)?))
}

/// Instantiate the declaration's binders before reducing their associated members.
/// VM frame arguments and native specializations share this exact boundary.
pub(crate) fn instantiate_witness(
    declarations: &impl DeclarationLookup,
    substitution: &TypeSubstitution,
    ty: &TypeRef,
) -> VbcResult<TypeRef> {
    let mut result = substitution.apply(ty)?;
    normalize_projections(declarations, &mut result)?;
    Ok(result)
}

/// Normalize declaration-backed associated assignments throughout a witness.
/// An unresolved member stays explicit for the consumer that requires it.
/// Bound both depth and expansion work, including branching recursive families.
pub(crate) fn normalize_projections(declarations: &impl DeclarationLookup, ty: &mut TypeRef) -> VbcResult<()> {
    fn normalize(
        declarations: &impl DeclarationLookup,
        ty: &mut TypeRef,
        depth: usize,
        remaining: &mut usize,
    ) -> VbcResult<()> {
        if depth > crate::mono::MAX_TYPE_REF_DEPTH || *remaining == 0 {
            return Err(VbcError::AssociatedType("associated-type expansion exceeds the type nesting or work limit".into()));
        }
        *remaining -= 1;
        let mut child = |ty: &mut TypeRef| normalize(declarations, ty, depth + 1, remaining);
        match ty {
            TypeRef::AssociatedProjection { base, qualifier, assoc, args } => {
                child(base)?;
                if let Some(qualifier) = qualifier {
                    qualifier.args.iter_mut().try_for_each(&mut child)?;
                }
                args.iter_mut().try_for_each(&mut child)?;
                if let Some(result) = projection_in(declarations, base, qualifier.as_ref(), assoc, args)? {
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
    normalize(declarations, ty, 0, &mut remaining)
}
