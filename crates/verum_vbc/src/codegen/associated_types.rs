//! Resolve carried associated bindings in the consumer's nominal pool.

use super::VbcCodegen;
use crate::{
    mono::TypeSubstitution,
    types::{TypeId, TypeRef},
};
use verum_common::{List, Set};

/// A written protocol qualifier must be elaborated as an exact application.
/// The current source lowerer cannot produce that witness, so it may not erase
/// the qualifier and publish an unqualified projection instead.
pub(super) fn require_projection_application_evidence(ty: &verum_ast::Type) -> super::CodegenResult<()> {
    if matches!(&ty.kind, verum_ast::TypeKind::Qualified { trait_ref, .. } if !trait_ref.segments.is_empty()) {
        return Err(super::CodegenError::with_span(super::CodegenErrorKind::InvalidSourceSignature(
            "qualified associated projection requires declaration-owned protocol application elaboration".into()), ty.span));
    }
    Ok(())
}

/// Source identities for fully specialized implementation heads. The ABI maps both
/// pointer-width integers to PTR; it cannot distinguish their implementations.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum ConcreteImplType {
    Resolved(TypeRef),
    PointerWidthInteger { signed: bool },
    Applied(TypeId, Vec<Self>),
}

impl ConcreteImplType {
    fn contradicts(&self, expected: &TypeRef) -> bool {
        match self {
            Self::Resolved(actual) => VbcCodegen::disjoint_nominal_types(actual, expected),
            // The source guarantees a scalar on every target. Its bit width is irrelevant.
            Self::PointerWidthInteger { .. } => matches!(expected, TypeRef::Instantiated { .. }),
            Self::Applied(base, args) => match expected {
                TypeRef::Concrete(id) => base != id,
                TypeRef::Instantiated {
                    base: other,
                    args: required,
                } => {
                    base != other
                        || args
                            .iter()
                            .zip(required.iter())
                            .any(|(a, b)| a.contradicts(b))
                }
                _ => false,
            },
        }
    }
}

impl VbcCodegen {
    /// Capture the nominal declarations a foreign implementor will need when
    /// checking inherited default signatures and bodies. Resolve them as
    /// ordinary signatures in this file, including their generic binders.
    pub(super) fn retain_default_method_types(&mut self, files: &[&verum_ast::Module]) {
        use verum_ast::{ItemKind, TypeDeclBody, Visitor};
        use verum_common::{Map, Text};
        struct Names<'a> {
            cg: &'a VbcCodegen,
            scope: &'a std::collections::HashMap<String, u16>,
            ids: &'a [crate::types::TypeParamId],
            bindings: &'a mut Map<Text, Option<Text>>,
        }
        impl Visitor for Names<'_> {
            fn visit_type(&mut self, ty: &verum_ast::Type) {
                if let verum_ast::TypeKind::Path(path) = &ty.kind {
                    if let Some(TypeRef::Concrete(id)) = self.cg.semantic_formal_type(ty, self.scope, self.ids)
                        && let Some(descriptor) = self.cg.type_by_id(id)
                        && descriptor.kind != crate::types::TypeKind::Primitive
                        && let Some(owner) = self.cg.type_declaration_identity(descriptor)
                    {
                        self.bindings.insert(path.to_string().replace("::", ".").into(), Some(owner));
                    }
                } else {
                    verum_ast::visitor::walk_type(self, ty);
                }
            }
        }
        for file in files {
            let owner = self.source_file_owner(file, &self.config.module_name)
                .unwrap_or_else(|| self.config.module_name.clone());
            let previous = self.ctx.current_source_module.replace(owner.clone());
            let mut bindings = Map::new();
            for item in &file.items {
                let ItemKind::Type(ty) = &item.kind else { continue };
                if !matches!(ty.body, TypeDeclBody::Protocol(_)) { continue; }
                let protocol_owner = crate::module::qualify_module_name(&owner, ty.name.name.as_str());
                let Some(protocol) = self.protocol_registry.get(&protocol_owner) else { continue };
                let parent_names: Vec<_> = ty.generics.iter().filter_map(|parameter| {
                    use verum_ast::ty::GenericParamKind;
                    match &parameter.kind {
                        GenericParamKind::Type { name, .. } | GenericParamKind::HigherKinded { name, .. } =>
                            Some(name.name.to_string()),
                        _ => None,
                    }
                }).collect();
                for method in protocol.default_methods.values() {
                    let (mut scope, shadow) = Self::declaration_generic_param_maps(method, &parent_names, &parent_names);
                    let ids = Self::ordered_generic_param_ids(&scope, &shadow);
                    scope.extend(shadow);
                    Names { cg: self, scope: &scope, ids: &ids, bindings: &mut bindings }.visit_function(method);
                }
            }
            self.ctx.current_source_module = previous;
            if !bindings.is_empty() {
                let lexical = self.source_type_mounts.entry(owner.into()).or_default();
                for (name, binding) in bindings { lexical.entry(name).or_insert_with(|| binding.into()); }
            }
        }
    }

    fn concrete_impl_type(&self, ty: &verum_ast::Type, depth: usize) -> Option<ConcreteImplType> {
        use verum_ast::ty::{GenericArg, TypeKind};
        use verum_common::well_known_types::type_names;
        if depth >= 64 {
            return None;
        }
        if let Some(ty) = self.semantic_formal_type(ty, &std::collections::HashMap::new(), &[]) {
            return (!ty.is_generic()).then_some(ConcreteImplType::Resolved(ty));
        }
        match &ty.kind {
            TypeKind::Path(path) if path.is_single() => {
                let name = path.to_string();
                (type_names::is_pointer_width_integer_type(&name)
                    && self.nominal_type_id(&name) == Some(TypeId::PTR))
                .then(|| ConcreteImplType::PointerWidthInteger {
                    signed: type_names::is_signed_integer_type(&name),
                })
            }
            TypeKind::Generic { base, args } => {
                let ConcreteImplType::Resolved(TypeRef::Concrete(id)) =
                    self.concrete_impl_type(base, depth + 1)?
                else {
                    return None;
                };
                let args = args
                    .iter()
                    .map(|arg| match arg {
                        GenericArg::Type(arg) => self.concrete_impl_type(arg, depth + 1),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(ConcreteImplType::Applied(id, args))
            }
            _ => None,
        }
    }

    /// `Range<Int>::Item = Int` belongs to one instantiation, so it cannot be
    /// published as an unconditional associated binding on `Range<T>`.
    pub(super) fn record_concrete_impl_bindings(
        &mut self,
        implementation: &verum_ast::decl::ImplDecl,
        selected_items: &[&verum_ast::decl::ImplItem],
    ) -> Option<ConcreteImplType> {
        use verum_ast::decl::{ImplItemKind, ImplKind};
        let ImplKind::Protocol {
            for_type, protocol, ..
        } = &implementation.kind
        else {
            return None;
        };
        if !implementation.generics.is_empty() {
            return None;
        }
        if !matches!(&for_type.kind, verum_ast::ty::TypeKind::Generic { .. }) {
            return None;
        }
        let receiver = self.concrete_impl_type(for_type, 0)?;
        let protocol_name = protocol.to_string().replace("::", ".");
        let protocol = crate::types::ProtocolId(self.nominal_type_id(&protocol_name)?.0);
        let facts: std::collections::HashMap<_, _> = selected_items
            .iter()
            .filter_map(|item| {
                let ImplItemKind::Type { name, ty, .. } = &item.kind else {
                    return None;
                };
                Some((name.name.to_string(), self.concrete_impl_type(ty, 0)?))
            })
            .collect();
        self.concrete_impl_bindings
            .entry((receiver.clone(), protocol))
            .and_modify(|previous| {
                previous.retain(|member, binding| facts.get(member) == Some(binding))
            })
            .or_insert(facts);
        Some(receiver)
    }

    /// A method slot may cover several specializations of the same nominal receiver.
    /// Omit it only when one explicit associated equality conflicts with all their bindings.
    pub(super) fn default_binding_contradicts(
        &self,
        function: &verum_ast::FunctionDecl,
        receiver_name: &str,
        receiver: crate::types::TypeId,
        protocol: crate::types::ProtocolId,
        instance: Option<&ConcreteImplType>,
    ) -> bool {
        use verum_ast::ty::{TypeBoundKind, TypeKind, WherePredicateKind};
        let Some(clause) = &function.generic_where_clause else {
            return false;
        };
        let Some(descriptor) = self.type_by_id(receiver) else {
            return false;
        };
        let (mut scope, shadow) =
            self.function_generic_param_maps(function, Some(receiver_name), &[]);
        let declared_ids = Self::ordered_generic_param_ids(&scope, &shadow);
        scope.extend(shadow);

        clause.predicates.iter().any(|predicate| {
            let WherePredicateKind::Type { ty, bounds } = &predicate.kind else {
                return false;
            };
            let member = match &ty.kind {
                TypeKind::Qualified {
                    self_ty,
                    trait_ref,
                    assoc_name,
                } if trait_ref.segments.is_empty() && Self::ast_type_is_bare_self(self_ty) => {
                    &assoc_name.name
                }
                TypeKind::AssociatedType { base, assoc } if Self::ast_type_is_bare_self(base) => {
                    &assoc.name
                }
                _ => return false,
            };
            bounds.iter().any(|bound| {
                let TypeBoundKind::Equality(required) = &bound.kind else {
                    return false;
                };
                let Some(required) = self.semantic_formal_type(required, &scope, &declared_ids)
                else {
                    return false;
                };
                if let Some(facts) = instance.and_then(|instance| {
                    self.concrete_impl_bindings
                        .get(&(instance.clone(), protocol))
                }) {
                    return facts
                        .get(member.as_str())
                        .is_some_and(|actual| actual.contradicts(&required));
                }
                let mut implementations = descriptor
                    .protocols
                    .iter()
                    .filter(|p| p.protocol == protocol)
                    .peekable();
                implementations.peek().is_some()
                    && implementations.all(|p| {
                        let mut bindings = p
                            .associated_types
                            .iter()
                            .filter(|(name, _)| {
                                self.ctx.strings.get(name.0 as usize).map(String::as_str)
                                    == Some(member.as_str())
                            })
                            .peekable();
                        bindings.peek().is_some()
                            && bindings.all(|(_, ty)| Self::disjoint_nominal_types(ty, &required))
                    })
            })
        })
    }

    fn disjoint_nominal_types(left: &TypeRef, right: &TypeRef) -> bool {
        match (left, right) {
            (TypeRef::Concrete(a), TypeRef::Concrete(b))
            | (TypeRef::Concrete(a), TypeRef::Instantiated { base: b, .. })
            | (TypeRef::Instantiated { base: a, .. }, TypeRef::Concrete(b)) => a != b,
            (
                TypeRef::Instantiated { base: a, args: xs },
                TypeRef::Instantiated { base: b, args: ys },
            ) => {
                a != b
                    || xs
                        .iter()
                        .zip(ys.iter())
                        .any(|(x, y)| Self::disjoint_nominal_types(x, y))
            }
            _ => false,
        }
    }

    /// Instantiate declaration-owned associated bindings. A projection does
    /// not carry a protocol ID, so incompatible same-named bindings remain
    /// unknown; neither protocol iteration order nor a method spelling wins.
    pub(super) fn resolve_declared_associated_types(&self, ty: &TypeRef) -> Option<TypeRef> {
        fn resolve(
            codegen: &VbcCodegen,
            ty: &TypeRef,
            active: &mut Set<TypeRef>,
            depth: usize,
        ) -> Option<TypeRef> {
            // The depth bound also stops expanding cycles such as
            // Grow<T>.Argument -> Grow<Grow<T>>.Argument.
            if depth >= 64 {
                return None;
            }
            let nested = |ty, active: &mut Set<TypeRef>| resolve(codegen, ty, active, depth + 1);
            match ty {
                TypeRef::RecordFieldProjection { .. } => None,
                TypeRef::AssociatedProjection { base, qualifier, assoc, args } => {
                    if qualifier.is_some() || !args.is_empty() { return None; }
                    if !active.insert(ty.clone()) {
                        return None;
                    }
                    let result = (|| {
                        let base = nested(base, active)?;
                        let (id, args) = match &base {
                            TypeRef::Concrete(id) => (*id, &[][..]),
                            TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
                            _ => return None,
                        };
                        let descriptor = codegen.type_by_id(id)?;
                        if descriptor.type_params.len() != args.len() {
                            return None;
                        }
                        let substitution = TypeSubstitution::new(&descriptor.type_params, args);
                        let mut binding = None;
                        for implementation in &descriptor.protocols {
                            for (name, target) in &implementation.associated_types {
                                if codegen.ctx.strings.get(name.0 as usize).map(String::as_str)
                                    != Some(assoc.as_str())
                                {
                                    continue;
                                }
                                let target = substitution.apply(target).ok()?;
                                if binding.as_ref().is_some_and(|previous| previous != &target) {
                                    return None;
                                }
                                binding = Some(target);
                            }
                        }
                        resolve(codegen, &binding?, active, depth + 1)
                    })();
                    active.remove(ty);
                    result
                }
                TypeRef::Instantiated { base, args } => Some(TypeRef::Instantiated {
                    base: *base,
                    args: args
                        .iter()
                        .map(|arg| nested(arg, active))
                        .collect::<Option<List<_>>>()?
                        .into(),
                }),
                TypeRef::Function {
                    is_async,
                    params,
                    return_type,
                    contexts,
                } => Some(TypeRef::Function {
                    is_async: *is_async,
                    params: params
                        .iter()
                        .map(|param| nested(param, active))
                        .collect::<Option<List<_>>>()?
                        .into(),
                    return_type: Box::new(nested(return_type, active)?),
                    contexts: contexts.clone(),
                }),
                TypeRef::Reference {
                    inner,
                    mutability,
                    tier,
                } => Some(TypeRef::Reference {
                    inner: Box::new(nested(inner, active)?),
                    mutability: *mutability,
                    tier: *tier,
                }),
                TypeRef::Tuple(items) => Some(TypeRef::Tuple(
                    items
                        .iter()
                        .map(|item| nested(item, active))
                        .collect::<Option<List<_>>>()?
                        .into(),
                )),
                TypeRef::Array { element, length } => Some(TypeRef::Array {
                    element: Box::new(nested(element, active)?),
                    length: *length,
                }),
                TypeRef::Slice(inner) => Some(TypeRef::Slice(Box::new(nested(inner, active)?))),
                // Rank-2 signatures are not certified as ordinary callables.
                TypeRef::Rank2Function { .. } => None,
                TypeRef::Concrete(_) | TypeRef::Generic(_) | TypeRef::ConstValue(_) => {
                    Some(ty.clone())
                }
            }
        }
        resolve(self, ty, &mut Set::new(), 0)
    }
}
