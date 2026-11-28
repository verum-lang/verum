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

pub(crate) fn declared_members(protocol: &TypeDescriptor) -> Option<impl Iterator<Item = (&str, usize)>> {
    let source = protocol.source_declaration.as_ref()?;
    let body = match &source.body {
        verum_ast::TypeDeclBody::Protocol(body) | verum_ast::TypeDeclBody::Coinductive(body) => body,
        _ => return None,
    };
    Some(body.items.iter().filter_map(|item| match &item.kind {
        verum_ast::decl::ProtocolItemKind::Type { name, type_params, .. } => Some((name.as_str(), type_params.len())),
        _ => None,
    }))
}

/// A reference has its own complete storage representation. Operations that
/// inspect a semantic signature, including calls to user code, need its target.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WitnessUse { Signature, Storage, CallableContract }

impl WitnessUse {
    pub(crate) fn child_scope(self, ty: &TypeRef) -> Option<Self> {
        match ty {
            TypeRef::Reference { .. } if self == Self::Storage => None,
            TypeRef::Function { .. } | TypeRef::Rank2Function { .. } | TypeRef::Dynamic(_)
                if self == Self::Storage => Some(Self::Signature),
            _ => Some(self),
        }
    }
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
        let Some(mut members) = declared_members(protocol) else { return Ok(None); };
        let Some((_, arity)) = members.find(|(name, _)| *name == assoc) else { continue; };
        // An erased equation is still a declaration of this member. It cannot
        // make a competing protocol the unique owner of an unqualified name.
        if !protocol.type_params.is_empty() || arity != 0
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

pub(crate) fn instantiate_storage(
    declarations: &impl DeclarationLookup,
    substitution: &TypeSubstitution,
    ty: &TypeRef,
) -> VbcResult<TypeRef> {
    let mut result = substitution.apply(ty)?;
    normalize_for(declarations, &mut result, WitnessUse::Storage)?;
    Ok(result)
}

/// Normalize declaration-backed associated assignments throughout a witness.
/// An unresolved member stays explicit for the consumer that requires it.
/// Bound both depth and expansion work, including branching recursive families.
pub(crate) fn normalize_projections(declarations: &impl DeclarationLookup, ty: &mut TypeRef) -> VbcResult<()> {
    normalize_for(declarations, ty, WitnessUse::Signature)
}

pub(crate) fn normalize_for(declarations: &impl DeclarationLookup, ty: &mut TypeRef, scope: WitnessUse) -> VbcResult<()> {
    fn normalize(
        declarations: &impl DeclarationLookup,
        ty: &mut TypeRef,
        scope: WitnessUse,
        depth: usize,
        remaining: &mut usize,
    ) -> VbcResult<()> {
        if depth > crate::mono::MAX_TYPE_REF_DEPTH || *remaining == 0 {
            return Err(VbcError::AssociatedType("associated-type expansion exceeds the type nesting or work limit".into()));
        }
        *remaining -= 1;
        if scope == WitnessUse::CallableContract
            && let Some(owner) = ty.base_type_id().and_then(|id| declarations.descriptor(id))
            && owner.kind == crate::types::TypeKind::Alias
            && owner.source_declaration.is_none()
            && !matches!(owner.resource_discipline,
                verum_common::ResourceDiscipline::Affine | verum_common::ResourceDiscipline::Linear)
        {
            // Aliases with extra source obligations keep their nominal witness.
            // A plain alias forwards only its own explicit target and arguments.
            let args = match ty {
                TypeRef::Concrete(_) => &[][..],
                TypeRef::Instantiated { args, .. } => args.as_slice(),
                _ => unreachable!("only concrete and instantiated types have alias declarations"),
            };
            let target = owner.alias_target.as_ref().filter(|_| args.len() == owner.type_params.len())
                .ok_or_else(|| VbcError::AssociatedType("callable alias lacks its declared target or arguments".into()))?;
            if matches!(target, TypeRef::Concrete(TypeId::UNIT | TypeId::PTR))
                && owner.alias_target_name.and_then(|name| declarations.spelling(name))
                    .and_then(TypeId::from_well_known_scalar_name) != target.base_type_id()
            {
                return Err(VbcError::AssociatedType("callable alias lacks scalar declaration evidence".into()));
            }
            *ty = TypeSubstitution::new(&owner.type_params, args).apply(target)?;
            return normalize(declarations, ty, scope, depth + 1, remaining);
        }
        let Some(child_scope) = scope.child_scope(ty) else { return Ok(()); };
        let mut child = |ty: &mut TypeRef| normalize(declarations, ty, child_scope, depth + 1, remaining);
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
    normalize(declarations, ty, scope, 0, &mut remaining)
}
