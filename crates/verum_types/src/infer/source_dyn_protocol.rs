//! Recursive `dyn` uses carry an obligation until their complete source
//! declaration has been checked. A nominal placeholder alone is not authority.
use super::TypeChecker;
use crate::{Result, Type, TypeError, protocol::{ObjectSafetyError, Protocol, ProtocolBound, ProtocolChecker}};
use verum_ast::{Span, ty::{TypeBinding, TypeBound, TypeBoundKind}};
use verum_common::{List, Map, Text};

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
        let mut names = List::new();
        for bound in bounds {
            let TypeBoundKind::Protocol(path) = &bound.kind else { continue; };
            let lexical = self.path_to_string(path);
            let (ty, key) = self.resolve_qualified_type_with_parameter_key(path, span)?;
            let declaration = match &ty {
                Type::Named { path, .. } => self.path_to_string(path),
                Type::Generic { name, .. } => name.clone(),
                _ => key.clone().unwrap_or_else(|| lexical.clone()),
            };
            // The selected source declaration's key outranks a same-spelling
            // ambient protocol. A header may only satisfy its own exact key.
            let identity = key.clone().unwrap_or_else(|| declaration.clone());
            if let Some(index) = self.source_type_registration.headers.iter()
                .rposition(|header| header.identity == identity)
            {
                // Referencing an ancestor can publish a dependent declaration
                // before this obligation is discharged. It therefore requires
                // the existing source group's rollback journal.
                if index + 1 != self.source_type_registration.headers.len()
                    && !self.source_type_registration.pending.contains_key(&identity)
                {
                    return Err(TypeError::Other(format!(
                        "recursive dyn bound `{identity}` has no shared source registration transaction"
                    ).into()));
                }
                self.source_type_registration.headers[index].dyn_required.get_or_insert(span);
                names.push(identity);
                continue;
            }
            if !self.protocol_checker.read().has_protocol(&declaration)
                && let Some(metadata) = self.core_metadata.clone()
                && metadata.protocols.contains_key(&declaration)
            {
                self.register_stdlib_protocol_for_name(&declaration, &metadata);
            }
            let registry = self.protocol_checker.read();
            let selected = if registry.has_protocol(&identity) {
                identity
            } else if declaration.contains(".") && registry.has_protocol(&declaration) {
                declaration
            } else {
                return Err(TypeError::Other(format!(
                    "dyn bound `{identity}` requires a protocol declaration"
                ).into()));
            };
            registry.check_object_safety(&selected)
                .map_err(|errors| object_safety_error(&selected, errors))?;
            names.push(selected);
        }
        let mut bindings_map = Map::new();
        if let Some(bindings) = bindings {
            for binding in bindings {
                let ty = self.ast_to_type(&binding.ty)?;
                bindings_map.insert(binding.name.name.clone(), ty);
            }
        }
        Ok(Type::DynProtocol { bounds: names, bindings: bindings_map })
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
        fn referent(ty: &Type) -> Option<(&Type, bool, verum_ast::expr::ReferenceKind)> {
            use verum_ast::expr::ReferenceKind;
            match ty {
                Type::Reference { inner, mutable } => {
                    Some((inner, *mutable, ReferenceKind::Managed))
                }
                Type::CheckedReference { inner, mutable } => {
                    Some((inner, *mutable, ReferenceKind::Checked))
                }
                Type::UnsafeReference { inner, mutable } => {
                    Some((inner, *mutable, ReferenceKind::Unsafe))
                }
                _ => None,
            }
        }
        if let Some((inner, mutable, kind)) = referent(expected) {
            return referent(actual).is_some_and(|(source, writable, tier)| {
                tier == kind
                    && (!mutable || writable)
                    && self.check_protocol_coercion_with(source, inner, registry)
            });
        }

        if let Some(required) = self.protocol_object_bounds(expected, registry) {
            if required.is_empty() {
                return false;
            }
            let declared = match actual {
                Type::Var(var) => Some(self.get_type_var_bounds(var)),
                _ => self.protocol_object_bounds(actual, registry),
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
                || self.protocol_object_bounds(target, registry).is_some()
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

    fn protocol_object_bounds(
        &self,
        ty: &Type,
        registry: &ProtocolChecker,
    ) -> Option<List<ProtocolBound>> {
        if let Type::DynProtocol { bounds, bindings } = ty {
            let mut required = List::new();
            for name in bounds {
                let protocol = registry.get_protocol_definition(name.as_str())?;
                if !protocol.type_params.is_empty() {
                    return None; // The erased spelling cannot supply an omitted positional argument.
                }
                required.push(ProtocolBound::simple(Self::text_to_path(name)));
            }
            for (name, value) in bindings {
                let mut owners = required.iter_mut().filter(|bound| {
                    registry
                        .lookup_associated_type_by_path(&bound.protocol, name)
                        .is_some()
                });
                let owner = owners.next()?;
                if owners.next().is_some() {
                    return None;
                }
                owner.associated_types.insert(name.clone(), value.clone());
            }
            return Some(required);
        }
        let (name, args) = self.protocol_carrier_application(ty)?;
        let declaration = registry.get_protocol_definition(name.as_str())?;
        if declaration.type_params.len() != args.len() {
            return None;
        }
        Some(
            [ProtocolBound::positive(
                Self::text_to_path(&name),
                args.iter().cloned().collect(),
            )]
            .into_iter()
            .collect(),
        )
    }

    pub(super) fn check_pending_protocol_object_safety(
        &self,
        protocol: &Protocol,
        required: Option<Span>,
    ) -> Result<()> {
        if required.is_some() {
            self.protocol_checker.read().check_definition_object_safety(protocol)
                .map_err(|errors| object_safety_error(&protocol.name, errors))?;
        }
        Ok(())
    }
}
