//! Recursive `dyn` uses carry an obligation until their complete source
//! declaration has been checked. A nominal placeholder alone is not authority.
use super::TypeChecker;
use crate::{Result, Type, TypeError, protocol::{ObjectSafetyError, Protocol}};
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
