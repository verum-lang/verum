//! Recursive `dyn` uses carry an obligation until their complete source
//! declaration has been checked. A nominal placeholder alone is not authority.
use super::TypeChecker;
use crate::{Result, Type, TypeError, protocol::{ObjectSafetyError, Protocol, ProtocolBound, ProtocolChecker, ProtocolConstraints}};
use verum_ast::{GenericArg, Span, ty::{TypeBinding, TypeBound, TypeBoundKind, TypeKind}};
use verum_common::{List, Map, Set, Text};

fn binding_error(span: Span, message: impl Into<Text>) -> TypeError {
    TypeError::OtherWithCodeSpanned { code: "E405".into(), msg: message.into(), span }
}

fn require_object_result_bindings(
    protocol: &Protocol,
    available: &Set<Text>,
    span: Span,
    registry: &ProtocolChecker,
) -> Result<()> {
    let required = registry.check_definition_object_safety(protocol)
        .map_err(|errors| object_safety_error(&protocol.name, errors))?;
    let mut missing: List<_> = required.into_iter().filter(|name| !available.contains(name)).collect();
    missing.sort();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(binding_error(span, format!("dynamic protocol `{}` requires explicit associated type bindings for {}",
            protocol.name, missing.join(", "))))
    }
}

/// A binding refers to a single ordinary member of one erased protocol.
fn bind_object_members(
    associated: &Map<Text, crate::protocol::AssociatedType>,
    bindings: &Map<Text, Type>,
    claimed: &mut Set<Text>,
    span: Span,
) -> Result<Set<Text>> {
    let mut matched = Set::new();
    for name in bindings.keys() {
        if let Some(member) = associated.get(name) {
            if !matches!(member.kind, crate::advanced_protocols::AssociatedTypeKind::Regular)
                || !member.binders.local.is_empty()
            {
                return Err(binding_error(span, format!("associated family `{name}` requires its declared arguments")));
            }
            if !claimed.insert(name.clone()) {
                return Err(binding_error(span, format!("multiple dynamic protocol bounds declare associated type `{name}`")));
            }
            matched.insert(name.clone());
        }
    }
    Ok(matched)
}

fn object_safety_error(name: &Text, errors: List<ObjectSafetyError>) -> TypeError {
    let mut messages: List<Text> = errors.iter().map(|error| error.to_string().into()).collect();
    messages.sort();
    TypeError::Other(format!(
        "Protocol '{name}' is not object-safe and cannot be used with `dyn`: {}",
        messages.join(", "),
    ).into())
}

impl TypeChecker {
    pub(super) fn elaborate_dyn_protocol_type(
        &mut self,
        bounds: &[TypeBound],
        bindings: Option<&[TypeBinding]>,
        span: Span,
    ) -> Result<Type> {
        let mut applications = List::new();
        let mut binding_types = Map::new();
        for binding in bindings.into_iter().flatten() {
            let ty = self.ast_to_type(&binding.ty)?;
            if binding_types.insert(binding.name.name.clone(), ty).is_some() {
                return Err(binding_error(binding.name.span, format!("duplicate dynamic associated type binding `{}`", binding.name)));
            }
        }
        let mut bound_members = Set::new();
        for bound in bounds {
            let (path, raw_args) = match &bound.kind {
                TypeBoundKind::Protocol(path) => (path, &[][..]),
                TypeBoundKind::GenericProtocol(ty) => match &ty.kind {
                    TypeKind::Generic { base, args } => match &base.kind {
                        TypeKind::Path(path) => (path, args.as_slice()),
                        _ => return Err(binding_error(bound.span, "dynamic application requires a declared protocol path")),
                    },
                    _ => return Err(binding_error(bound.span, "invalid dynamic protocol application")),
                },
                _ => return Err(binding_error(bound.span, "dynamic bound requires a positive protocol declaration")),
            };
            let lexical = self.path_to_string(path);
            let (ty, key) = self.resolve_qualified_type_with_parameter_key(path, bound.span)?;
            let declaration = match &ty {
                Type::Named { path, .. } => self.path_to_string(path),
                Type::Generic { name, .. } => name.clone(),
                _ => key.clone().unwrap_or_else(|| lexical.clone()),
            };
            let identity = key.unwrap_or_else(|| declaration.clone());
            let header = self.source_type_registration.headers.iter()
                .rposition(|header| header.identity == identity);
            let selected = if let Some(index) = header {
                if index + 1 != self.source_type_registration.headers.len()
                    && !self.source_type_registration.pending.contains_key(&identity)
                {
                    return Err(binding_error(bound.span, format!(
                        "recursive dyn bound `{identity}` has no shared source registration transaction"
                    )));
                }
                identity
            } else {
                if !self.protocol_checker.read().has_protocol(&declaration)
                    && let Some(metadata) = self.core_metadata.clone()
                    && metadata.protocols.contains_key(&declaration)
                {
                    self.register_stdlib_protocol_for_name(&declaration, &metadata);
                }
                let registry = self.protocol_checker.read();
                if registry.has_protocol(&identity) { identity }
                else if declaration.contains(".") && registry.has_protocol(&declaration) { declaration }
                else { return Err(binding_error(bound.span, format!("dyn bound `{identity}` requires a protocol declaration"))); }
            };

            let mut application = ProtocolBound::object(&selected);
            let mut positional = List::new();
            for arg in raw_args {
                if let GenericArg::Binding(binding) = arg {
                    let value = self.ast_to_type(&binding.ty)?;
                    if application.associated_types.insert(binding.name.name.clone(), value).is_some()
                        || binding_types.contains_key(&binding.name.name)
                    {
                        return Err(binding_error(binding.span, format!("duplicate dynamic associated type binding `{}`", binding.name)));
                    }
                } else { positional.push(arg.clone()); }
            }
            let mut use_path = application.protocol.clone();
            use_path.span = bound.span;
            application.args = self.protocol_application_arguments(&use_path, &positional, None)?;

            // Legacy equality-only syntax has a single list for all bounds. Select
            // its unique declaring application, then store the equality on that owner.
            let mut bind_members = |associated: &Map<_, _>| -> Result<Set<Text>> {
                let matched = bind_object_members(associated, &binding_types, &mut bound_members, span)?;
                let mut claimed = Set::new();
                let direct = bind_object_members(associated, &application.associated_types, &mut claimed, span)?;
                if direct.len() != application.associated_types.len() {
                    return Err(binding_error(span, format!("dynamic protocol `{selected}` does not declare the named associated type")));
                }
                for name in &matched {
                    application.associated_types.insert(name.clone(), binding_types[name].clone());
                }
                Ok(application.associated_types.keys().cloned().collect())
            };
            let constraints = if let Some(index) = header {
                let header = &mut self.source_type_registration.headers[index];
                let available = bind_members(&header.associated)?;
                header.dyn_required.push((span, available));
                header.constraints.clone()
            } else {
                let registry = self.protocol_checker.read();
                let protocol = registry.get_protocol_definition(&selected)
                    .ok_or_else(|| binding_error(span, format!("dynamic protocol `{selected}` has no declaration")))?;
                let available = bind_members(&protocol.associated_types)?;
                require_object_result_bindings(protocol, &available, span, &registry)?;
                protocol.constraints.clone()
            };
            self.check_dynamic_arguments(&application, positional.len(), &constraints, bound.span)?;
            applications.push(application);
        }
        if let Some(name) = binding_types.keys().find(|name| !bound_members.contains(*name)) {
            return Err(binding_error(span, format!("no selected dynamic protocol declares associated type `{name}`")));
        }
        Ok(Type::DynProtocol { bounds: applications })
    }

    fn check_dynamic_arguments(
        &mut self,
        application: &ProtocolBound,
        supplied: usize,
        constraints: &ProtocolConstraints,
        span: Span,
    ) -> Result<()> {
        let ProtocolConstraints::Declared { self_type, defaults, parameter_bounds, .. } = constraints else {
            if application.args.is_empty() { return Ok(()); }
            return Err(binding_error(span, "dynamic application lacks its declaration-owned telescope"));
        };
        if defaults.iter().skip(supplied).flatten().any(|ty| ty.free_vars().contains(self_type)) {
            return Err(binding_error(span, "an erased Self cannot supply a dynamic protocol's default argument"));
        }
        // The hidden implementing type supplies no assumptions while checking the
        // argument domain. Its actual witness is selected when a value is erased.
        let subject = Type::Var(crate::ty::TypeVar::fresh());
        let scope = constraints.application(&subject, &application.args)
            .map_err(|message| binding_error(span, message))?;
        let requirements = parameter_bounds.iter().filter(|clause| !clause.bounds.is_empty())
            .map(|clause| clause.try_apply_subst(&scope).map_err(|message| binding_error(span, message)))
            .collect::<Result<_>>()?;
        self.prove_protocol_application_requirements(requirements, span)
    }

    /// Erasure requires evidence for the selected declaration and its indices.
    /// Unification only knows the types' shapes and cannot supply this proof.
    pub(super) fn check_protocol_coercion(&self, actual: &Type, expected: &Type) -> bool {
        self.check_protocol_coercion_with(actual, expected, &self.protocol_checker.read())
    }

    fn check_protocol_coercion_with(
        &self,
        actual: &Type,
        expected: &Type,
        registry: &ProtocolChecker,
    ) -> bool {
        if let Some((inner, mutable, kind)) = expected.reference_parts() {
            return actual.reference_parts().is_some_and(|(source, writable, tier)| {
                tier == kind
                    && (!mutable || writable)
                    && if mutable && registry.object_protocol_bounds(inner).is_none() {
                        // Writing a nominal wrapper can replace its payload. Erasing
                        // that payload would change the original owner's type.
                        registry.types_structurally_equal(source, inner)
                    } else {
                        self.check_protocol_coercion_with(source, inner, registry)
                    }
            });
        }

        if let Some(required) = registry.object_protocol_bounds(expected) {
            if required.is_empty() {
                return false;
            }
            if matches!(actual, Type::DynProtocol { .. }) {
                return registry.check_bounds(actual, &required).is_ok();
            }
            let declared = match actual {
                Type::Var(var) => Some(self.get_type_var_bounds(var)),
                _ => registry.object_protocol_bounds(actual),
            };
            if let Some(declared) = declared {
                return required.iter().all(|need| {
                    declared.iter().any(|have| {
                        !have.is_negative
                            && crate::ty::same_type_path(&have.protocol, &need.protocol)
                            && have.args.len() == need.args.len()
                            && have
                                .args
                                .iter()
                                .zip(&need.args)
                                .all(|(a, b)| registry.types_structurally_equal(a, b))
                            && need.associated_types.iter().all(|(name, value)| {
                                have.associated_types
                                    .get(name)
                                    .is_some_and(|ty| registry.types_structurally_equal(ty, value))
                            })
                    })
                });
            }
            return registry.check_bounds(actual, &required).is_ok();
        }

        // Retain direct protocol-valued arguments of an existing nominal wrapper.
        let Some((owner, target_args)) = self.protocol_carrier_application(expected) else {
            return false;
        };
        let Some((carrier, source_args)) = self.protocol_carrier_application(actual) else {
            return false;
        };
        if carrier != owner || target_args.is_empty() || source_args.len() != target_args.len() {
            return false;
        }
        source_args.iter().zip(target_args).all(|(source, target)| {
            source == target
                || registry.object_protocol_bounds(target).is_some()
                    && self.check_protocol_coercion_with(source, target, registry)
        })
    }

    fn protocol_carrier_application<'a>(&self, ty: &'a Type) -> Option<(Text, &'a [Type])> {
        match ty {
            Type::Named { path, args } => Some((self.path_to_string(path), args)),
            Type::Generic { name, args } => Some((name.clone(), args)),
            _ => None,
        }
    }

    pub(super) fn check_pending_protocol_object_safety(
        &self,
        protocol: &Protocol,
        required: super::source_protocol_header::DynObligations,
    ) -> Result<()> {
        let registry = self.protocol_checker.read();
        for (span, available) in required {
            require_object_result_bindings(protocol, &available, span, &registry)?;
        }
        Ok(())
    }
}
