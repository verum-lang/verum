//! Declaration-owned equality premises refine body queries, not source headers.
use super::{CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen};
use crate::types::{TypeRef, MAX_TYPE_REF_DEPTH, MAX_TYPE_REF_NODES};
use verum_common::{List, Maybe};

#[derive(Default)]
pub(super) struct DeclarationEqualities {
    /// Definitions belonging to the one implementation selected for this body.
    /// A different protocol on the same receiver can own a homonymous member.
    members: List<(TypeRef, TypeRef)>,
    rules: List<(TypeRef, TypeRef)>,
}

#[cfg(test)]
#[path = "../../tests/codegen/declaration_equalities.rs"]
mod tests;

fn invalid(message: impl Into<verum_common::Text>) -> CodegenError {
    CodegenError::new(CodegenErrorKind::InvalidSourceSignature(message.into().to_string()))
}

impl DeclarationEqualities {
    fn with_members(codegen: &VbcCodegen, members: List<(TypeRef, TypeRef)>) -> CodegenResult<Self> {
        if members.len() > MAX_TYPE_REF_NODES {
            return Err(invalid("associated member definitions exceed the premise limit"));
        }
        let mut seen = verum_common::Set::new();
        for (subject, _) in &members {
            if !seen.insert(subject) { return Err(invalid("duplicate associated member definition")); }
        }
        let scope = Self { members, rules: List::new() };
        for (subject, _) in &scope.members {
            scope.normalize(codegen, subject)?;
        }
        Ok(scope)
    }

    fn normalize(&self, codegen: &VbcCodegen, ty: &TypeRef) -> CodegenResult<TypeRef> {
        Normalizer {
            codegen, equalities: self, active: List::new(), remaining: MAX_TYPE_REF_NODES,
        }.resolve(ty, 0, 0)
    }

    fn add(&mut self, codegen: &VbcCodegen, left: TypeRef, right: TypeRef) -> CodegenResult<()> {
        let mut pending = List::new();
        pending.push((left, right));
        let mut remaining = MAX_TYPE_REF_NODES;
        while let Some((left, right)) = pending.pop() {
            remaining = remaining.checked_sub(1).ok_or_else(|| invalid("declared type equality exceeds the component limit"))?;
            let left = self.normalize(codegen, &left)?;
            let right = self.normalize(codegen, &right)?;
            if left == right { continue; }
            // A projection can equal its receiver without a recursive value
            // layout. Prefer that direction over substituting the receiver.
            let equation = match (&left, &right) {
                (TypeRef::AssociatedProjection { .. }, _) => Some((left.clone(), right.clone())),
                (_, TypeRef::AssociatedProjection { .. }) => Some((right.clone(), left.clone())),
                (TypeRef::Generic(a), TypeRef::Generic(b)) if a.0 < b.0 => Some((right.clone(), left.clone())),
                (TypeRef::Generic(_), _) => Some((left.clone(), right.clone())),
                (_, TypeRef::Generic(_)) => Some((right.clone(), left.clone())),
                _ => None,
            };
            if let Some(equation) = equation {
                if self.rules.len() >= MAX_TYPE_REF_NODES {
                    return Err(invalid("declared type equality exceeds the premise limit"));
                }
                self.rules.push(equation);
                // Refuse recursive expansion and contradictions before any body
                // consumer can use a partially collected set of premises.
                for (subject, target) in &self.rules {
                    if self.normalize(codegen, subject)? != self.normalize(codegen, target)? {
                        return Err(invalid("conflicting declared type equalities"));
                    }
                }
                continue;
            }
            match (&left, &right) {
                (TypeRef::Instantiated { base: a, args: aa }, TypeRef::Instantiated { base: b, args: ba })
                    if a == b && aa.len() == ba.len() => pending.extend(aa.iter().cloned().zip(ba.iter().cloned())),
                (TypeRef::Tuple(a), TypeRef::Tuple(b)) if a.len() == b.len() =>
                    pending.extend(a.iter().cloned().zip(b.iter().cloned())),
                (TypeRef::Reference { inner: a, mutability: am, tier: at },
                 TypeRef::Reference { inner: b, mutability: bm, tier: bt }) if am == bm && at == bt =>
                    pending.push((a.as_ref().clone(), b.as_ref().clone())),
                (TypeRef::Array { element: a, length: al }, TypeRef::Array { element: b, length: bl }) if al == bl =>
                    pending.push((a.as_ref().clone(), b.as_ref().clone())),
                (TypeRef::Slice(a), TypeRef::Slice(b)) => pending.push((a.as_ref().clone(), b.as_ref().clone())),
                (TypeRef::Function { is_async: aa, params: ap, return_type: ar, contexts: ac },
                 TypeRef::Function { is_async: ba, params: bp, return_type: br, contexts: bc })
                    if aa == ba && ac == bc && ap.len() == bp.len() => {
                    pending.extend(ap.iter().cloned().zip(bp.iter().cloned()));
                    pending.push((ar.as_ref().clone(), br.as_ref().clone()));
                }
                // Quantified/existential subjects require their own binder
                // proof. Equality of their rendered shapes is not that proof.
                _ => {
                    let identity = |ty: &TypeRef| ty.base_type_id().and_then(|id| codegen.type_by_id(id))
                        .and_then(|declaration| codegen.type_declaration_identity(declaration));
                    return Err(invalid(format!("conflicting or unsupported declared type equality: {left:?} = {right:?}; declared owners {:?} and {:?}", identity(&left), identity(&right))));
                }
            }
        }
        Ok(())
    }
}

struct Normalizer<'a> {
    codegen: &'a VbcCodegen,
    equalities: &'a DeclarationEqualities,
    active: List<(TypeRef, u32)>,
    remaining: usize,
}

impl Normalizer<'_> {
    fn resolve(&mut self, ty: &TypeRef, binders: u32, depth: usize) -> CodegenResult<TypeRef> {
        self.remaining = self.remaining.checked_sub(1).ok_or_else(|| invalid("declared type equality exceeds the component limit"))?;
        if depth >= MAX_TYPE_REF_DEPTH {
            return Err(invalid("declared type equality exceeds the depth limit"));
        }
        if self.active.iter().any(|(subject, scope)| *scope == binders && subject == ty) {
            return Err(invalid("cyclic declared type equality"));
        }
        let ty = map_children(ty, binders, &mut |child, scope| self.resolve(child, scope, depth + 1))?;
        if matches!(&ty, TypeRef::Concrete(_) | TypeRef::Instantiated { .. })
            && ty.base_type_id().and_then(|id| self.codegen.type_by_id(id))
                .is_some_and(|declaration| declaration.kind == crate::types::TypeKind::Alias)
        {
            let expanded = self.codegen.expand_declared_alias(ty.clone())
                .ok_or_else(|| invalid("declared type equality requires a complete acyclic alias application"))?;
            return self.resolve(&expanded, binders, depth + 1);
        }
        let equalities = self.equalities;
        if let TypeRef::AssociatedProjection { .. } = &ty
            && let Some(value) = self.rewrite(&ty, binders, depth, &equalities.members)? {
            return Ok(value);
        }
        if let Some(resolved) = self.codegen.declared_associated_projection_target(&ty)
            && resolved != ty {
            return self.resolve(&resolved, binders, depth + 1);
        }
        Ok(self.rewrite(&ty, binders, depth, &equalities.rules)?.unwrap_or(ty))
    }

    fn rewrite(
        &mut self, ty: &TypeRef, binders: u32, depth: usize, rules: &[(TypeRef, TypeRef)],
    ) -> CodegenResult<Maybe<TypeRef>> {
        let mut selected = None;
        for (subject, target) in rules {
            if !matches!((ty, subject),
                (TypeRef::Generic(_), TypeRef::Generic(_)) |
                (TypeRef::AssociatedProjection { .. }, TypeRef::AssociatedProjection { .. })) { continue; }
            let lift = |ty| crate::mono::lift_free_parameters(ty, binders)
                .map_err(|error| invalid(error.to_string()));
            // Normalize the subject's children without applying its own rule
            // at the root; a later premise may refine a projection's base.
            let subject = map_children(&lift(subject)?, binders,
                &mut |child, scope| self.resolve(child, scope, depth + 1))?;
            if &subject != ty { continue; }
            if self.active.iter().any(|(subject, scope)| *scope == binders && subject == ty) {
                return Err(invalid("cyclic declared type equality"));
            }
            self.active.push((ty.clone(), binders));
            let result = self.resolve(&lift(target)?, binders, depth + 1);
            self.active.pop();
            let result = result?;
            if selected.as_ref().is_some_and(|previous| previous != &result) {
                return Err(invalid("conflicting declared type equalities"));
            }
            selected = Some(result);
        }
        Ok(selected)
    }
}

pub(super) fn map_children(
    ty: &TypeRef, binders: u32,
    map: &mut impl FnMut(&TypeRef, u32) -> CodegenResult<TypeRef>,
) -> CodegenResult<TypeRef> {
    Ok(match ty {
        TypeRef::Instantiated { base, args } => TypeRef::Instantiated {
            base: *base, args: args.iter().map(|ty| map(ty, binders)).collect::<CodegenResult<_>>()?,
        },
        TypeRef::Function { is_async, params, return_type, contexts } => TypeRef::Function {
            is_async: *is_async,
            params: params.iter().map(|ty| map(ty, binders)).collect::<CodegenResult<_>>()?,
            return_type: Box::new(map(return_type, binders)?), contexts: contexts.clone(),
        },
        TypeRef::Rank2Function { is_async, type_param_count, params, return_type, contexts } => {
            let binders = binders.checked_add(u32::from(*type_param_count))
                .ok_or_else(|| invalid("declared type equality binder overflow"))?;
            TypeRef::Rank2Function {
                is_async: *is_async, type_param_count: *type_param_count,
                params: params.iter().map(|ty| map(ty, binders)).collect::<CodegenResult<_>>()?,
                return_type: Box::new(map(return_type, binders)?), contexts: contexts.clone(),
            }
        }
        TypeRef::Dynamic(object) => {
            let binders = binders.checked_add(1).ok_or_else(|| invalid("declared type equality binder overflow"))?;
            TypeRef::Dynamic(object.try_map(|ty| map(ty, binders))?)
        }
        TypeRef::Reference { inner, tier, mutability } => TypeRef::Reference {
            inner: Box::new(map(inner, binders)?), tier: *tier, mutability: *mutability,
        },
        TypeRef::Tuple(parts) => TypeRef::Tuple(parts.iter().map(|ty| map(ty, binders)).collect::<CodegenResult<_>>()?),
        TypeRef::Array { element, length } => TypeRef::Array { element: Box::new(map(element, binders)?), length: *length },
        TypeRef::Slice(inner) => TypeRef::Slice(Box::new(map(inner, binders)?)),
        TypeRef::AssociatedProjection { base, qualifier, assoc, args } => TypeRef::AssociatedProjection {
            base: Box::new(map(base, binders)?), assoc: assoc.clone(),
            qualifier: qualifier.as_ref().map(|application| -> CodegenResult<_> { Ok(crate::types::ProtocolApplication {
                protocol: application.protocol,
                args: application.args.iter().map(|ty| map(ty, binders)).collect::<CodegenResult<_>>()?,
            }) }).transpose()?,
            args: args.iter().map(|ty| map(ty, binders)).collect::<CodegenResult<_>>()?,
        },
        TypeRef::Concrete(_) | TypeRef::Generic(_) | TypeRef::RecordFieldProjection { .. }
        | TypeRef::ConstValue(_) | TypeRef::ConstSequence(_) => ty.clone(),
    })
}

impl VbcCodegen {
    /// Resolve an unqualified projection in the binder's own list of bounds.
    /// After substitution the caller's nominal type may implement unrelated
    /// protocols that declare a member with the same name.
    pub(super) fn qualify_bound_projections(
        &self, ty: &TypeRef, bounds: &verum_common::Map<TypeRef, List<super::protocol_results::BoundProtocol>>,
    ) -> Maybe<TypeRef> {
        fn visit(cg: &VbcCodegen, ty: &TypeRef, bounds: &verum_common::Map<TypeRef, List<super::protocol_results::BoundProtocol>>,
            depth: usize, remaining: &mut usize) -> CodegenResult<TypeRef> {
            *remaining = remaining.checked_sub(1).ok_or_else(|| invalid("member owner exceeds component limit"))?;
            if depth >= MAX_TYPE_REF_DEPTH { return Err(invalid("member owner exceeds depth limit")); }
            let mut ty = map_children(ty, 0, &mut |part, binders| {
                if binders == 0 { visit(cg, part, bounds, depth + 1, remaining) } else { Ok(part.clone()) }
            })?;
            if let TypeRef::AssociatedProjection { base, qualifier: owner @ None, assoc, args } = &mut ty
                && args.is_empty() && let Some(requires) = bounds.get(base.as_ref()) {
                let declaring = cg.bound_member_owner(requires.clone(), base, assoc)
                    .ok_or_else(|| invalid("associated member has no unique owner in its declared bounds"))?;
                let (id, args) = match declaring {
                    TypeRef::Concrete(id) => (id, Vec::new()),
                    TypeRef::Instantiated { base, args } => (base, args),
                    _ => return Err(invalid("invalid associated member owner")),
                };
                *owner = Some(crate::types::ProtocolApplication { protocol: crate::types::ProtocolId(id.0), args });
            }
            Ok(ty)
        }
        let mut remaining = MAX_TYPE_REF_NODES;
        visit(self, ty, bounds, 0, &mut remaining).ok()
    }

    /// Normalize a selected method's bound before crossing into caller scope.
    /// The runtime method slot selects the retained implementation; neither a
    /// same-named method nor a same-numbered caller parameter is that witness.
    pub(super) fn callable_declaration_type(
        &self, function: &super::FunctionInfo, ty: &TypeRef,
    ) -> Maybe<TypeRef> {
        use super::formal_parameters::DeclarationTypeScope;
        use verum_ast::decl::{ImplItemKind, ImplKind};
        let descriptor = self.declared_callable_descriptors.get(&function.id)
            .or_else(|| self.functions.iter().find(|body| body.descriptor.id == function.id)
                .map(|body| &body.descriptor))?;
        let unrefined = || DeclarationEqualities::default().normalize(self, ty).ok();
        let mut pending = List::from_iter([(ty, 0usize)]);
        let mut remaining = MAX_TYPE_REF_NODES;
        let mut projected = false;
        while let Some((part, depth)) = pending.pop() {
            remaining = remaining.checked_sub(1)?;
            if depth >= MAX_TYPE_REF_DEPTH { return None; }
            if matches!(part, TypeRef::AssociatedProjection { .. }) { projected = true; break; }
            part.for_each_child(|child| pending.push((child, depth + 1)));
        }
        if !projected { return unrefined(); }
        let Some(receiver) = descriptor.source_receiver.as_ref() else { return unrefined(); };
        let Some(parent) = descriptor.parent_type.and_then(|id| self.type_by_id(id))
            else { return unrefined(); };
        let owner = match descriptor.origin_module {
            Some(id) => self.ctx.strings.get(id.0 as usize)?.as_str(),
            None => self.config.module_name.as_str(),
        };
        let mut sources = verum_common::Set::new();
        for row in &parent.protocols {
            if row.methods.contains(&function.id.0) {
                sources.insert(row.source_implementation.as_ref()?);
            }
        }
        if sources.is_empty() { return unrefined(); }
        if sources.len() != 1 { return None; }
        let source = sources.iter().next()?;
        if source.origin_module() != owner { return None; }
        let implementation = source.declaration().ok()?;
        let ImplKind::Protocol { for_type, .. } = &implementation.kind else { return None; };
        let scope = self.callable_owner_generic_scope(function, descriptor)?;
        let ids: List<_> = scope.values().copied().map(crate::types::TypeParamId).collect();
        let empty = super::source_substitution::SourceTypeSubstitution::default();
        let lexical = DeclarationTypeScope { owner, substitution: &empty, receiver: None };
        let head = self.semantic_formal_type_in_declaration(
            for_type, &scope, &ids, &Default::default(), &lexical)?;
        if &head != receiver { return None; }
        let receiver = super::ImplReceiverWitness { ty: head, semantic: true };
        let lexical = DeclarationTypeScope { receiver: Some(&receiver), ..lexical };
        let mut definitions = List::new();
        for item in &implementation.items {
            let ImplItemKind::Type { name, ty, type_params } = &item.kind else { continue; };
            if !type_params.is_empty() { continue; }
            let target = self.semantic_formal_type_in_declaration(
                ty, &scope, &ids, &Default::default(), &lexical)?;
            definitions.push((TypeRef::AssociatedProjection {
                base: Box::new(receiver.ty.clone()), qualifier: None,
                assoc: name.name.to_string(), args: Default::default(),
            }, target));
        }
        let equalities = DeclarationEqualities::with_members(self, definitions).ok()?;
        let result = equalities.normalize(self, ty).ok()?;
        let mut bounds = verum_common::Map::new();
        self.visit_declared_protocol_bounds(&implementation.generics, implementation.generic_where_clause.as_ref(),
            &scope, &ids, &lexical, |subject| Some(subject.clone()),
            |subject, bound| bounds.entry(subject.clone()).or_insert_with(List::new).push(bound));
        let result = self.qualify_bound_projections(&result, &bounds)?;
        super::formal_parameters::resolved(&result, &function.type_param_ids, 0).then_some(result)
    }

    /// An implementation may bind a member to a parameter absent from its
    /// nominal receiver (for example, a callback's result parameter). Such an
    /// equation belongs to this declaration's body scope, not the unconditional
    /// associated-type table in the nominal descriptor.
    pub(super) fn implementation_associated_equalities(
        &self, implementation: &verum_ast::decl::ImplDecl,
        receiver: &super::ImplReceiverWitness,
        scope: &std::collections::HashMap<String, u16>,
    ) -> CodegenResult<List<(TypeRef, TypeRef)>> {
        use super::formal_parameters::DeclarationTypeScope;
        use verum_ast::decl::ImplItemKind;
        let substitution = self.implementation_value_scope(implementation, scope)?;
        let declaration = DeclarationTypeScope {
            owner: self.ctx.current_source_module.as_deref().unwrap_or(&self.config.module_name),
            substitution: &substitution, receiver: Some(receiver),
        };
        let ids = Self::ordered_generic_param_ids(scope, &Default::default());
        let mut equations = List::new();
        for item in self.selected_impl_items(implementation)? {
            let ImplItemKind::Type { name, ty, type_params } = &item.kind else { continue; };
            // A family needs its applied local binders; it is never a constant
            // member merely because this caller has not supplied arguments.
            if !type_params.is_empty() { continue; }
            if !receiver.semantic {
                return Err(invalid("associated implementation equation has no declared receiver"));
            }
            let target = self.semantic_formal_type_in_declaration(
                ty, scope, &ids, &Default::default(), &declaration,
            ).ok_or_else(|| invalid("associated implementation equation has no declared target"))?;
            equations.push((TypeRef::AssociatedProjection {
                base: Box::new(receiver.ty.clone()), qualifier: None,
                assoc: name.name.to_string(), args: Default::default(),
            }, target));
        }
        Ok(equations)
    }

    pub(super) fn implementation_equalities(&self) -> CodegenResult<DeclarationEqualities> {
        DeclarationEqualities::with_members(self, self.current_impl_associated_equalities.clone())
    }

    pub(super) fn deferred_protocol_result(&self, ty: TypeRef) -> Maybe<TypeRef> {
        if matches!(ty, TypeRef::Concrete(_) | TypeRef::Generic(_)) { return Some(ty); }
        let Some(scope) = &self.current_deferred_impl else { return Some(ty) };
        let Some(application) = &scope.protocol else { return Some(ty) };
        let root = self.declared_protocol_application(application)?;
        let mut bounds = verum_common::Map::new();
        bounds.insert(scope.receiver.ty.clone(), List::from_iter([root.into()]));
        self.qualify_bound_projections(&ty, &bounds)
    }

    /// A parent's associated assignment may live in a different conditional
    /// implementation. Its own subject, arguments and premises must all match
    /// before the child method's body can use that assignment.
    pub(super) fn inherit_implementation_members(&mut self) {
        let Some(scope) = self.current_deferred_impl.as_ref().filter(|scope| scope.receiver.semantic) else { return };
        let Some(application) = scope.protocol.as_ref() else { return };
        let subject = scope.receiver.ty.clone();
        let Some(root) = self.declared_protocol_application(application) else { return };
        let roots = List::from_iter([root.clone().into()]);
        let Some(hierarchy) = self.protocol_hierarchy(roots.clone(), Some(&subject)) else { return };
        let mut inherited = List::new();
        for parent in &hierarchy {
            let Some(protocol) = parent.application.base_type_id().and_then(|id| self.type_by_id(id)) else { continue };
            if let Some(members) = crate::associated_types::declared_members(protocol) {
                inherited.extend(members.filter(|(_, arity)| *arity == 0).map(|(name, _)| name.to_owned()));
            }
        }
        inherited.sort_unstable();
        inherited.dedup();
        let mut remaining = MAX_TYPE_REF_NODES;
        for member in inherited {
            let projection = TypeRef::AssociatedProjection {
                base: Box::new(subject.clone()), qualifier: None, assoc: member.clone(), args: Vec::new(),
            };
            let Some(declaring) = self.bound_member_owner(roots.clone(), &subject, &member) else { continue };
            let (id, arguments) = match &declaring {
                TypeRef::Concrete(id) => (*id, &[][..]),
                TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
                _ => continue,
            };
            let known = self.current_type_equalities.members.iter()
                .find(|(known, _)| *known == projection).map(|(_, value)| value.clone());
            let value = match known {
                Some(value) => value,
                None if declaring != root => {
                    let Some(value) = self.proven_protocol_member(&subject, id, arguments, &member, 0, &mut remaining)
                        .filter(|value| *value != projection && self.declared_type_in_scope(value)) else { continue };
                    self.current_type_equalities.members.push((projection, value.clone()));
                    value
                }
                None => continue,
            };
            self.current_type_equalities.members.push((TypeRef::AssociatedProjection {
                base: Box::new(subject.clone()), assoc: member, args: Vec::new(),
                qualifier: Some(crate::types::ProtocolApplication { protocol: crate::types::ProtocolId(id.0), args: arguments.to_vec() }),
            }, value));
        }
    }

    pub(super) fn collect_declaration_equalities(
        &self, clause: Maybe<&verum_ast::ty::WhereClause>,
        scope: &std::collections::HashMap<String, u16>,
        declared_ids: &[crate::types::TypeParamId], output: &mut DeclarationEqualities,
    ) -> CodegenResult<()> {
        use verum_ast::ty::{TypeBoundKind, WherePredicateKind};
        for predicate in clause.into_iter().flat_map(|clause| &clause.predicates) {
            let WherePredicateKind::Type { ty, bounds } = &predicate.kind else { continue; };
            for bound in bounds {
                let TypeBoundKind::Equality(target) = &bound.kind else { continue; };
                let left = self.semantic_formal_type(ty, scope, declared_ids)
                    .ok_or_else(|| invalid("declared type equality subject has no source declaration"))?;
                let right = self.semantic_formal_type(target, scope, declared_ids)
                    .ok_or_else(|| invalid("declared type equality target has no source declaration"))?;
                output.add(self, left, right)?;
            }
        }
        Ok(())
    }

    /// Body uses may refine a declaration under this callable's premises.
    /// Published parameters and results retain their original source telescope.
    pub(super) fn body_declaration_type(&self, ty: &TypeRef) -> Maybe<TypeRef> {
        self.current_type_equalities.normalize(self, ty).ok()
    }
}
