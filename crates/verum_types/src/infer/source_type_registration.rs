//! Source declaration loads publish success only after recursive loading closes.
use crate::{Result, Type, TypeError};
use verum_ast::TypeDecl;
use verum_common::{List, Map, Text};

#[derive(Debug, Default)]
pub(crate) struct SourceTypeRegistration {
    /// Lexical source declarations available while preparing one module.
    pub local_declarations: List<(Text, List<TypeDecl>)>,
    pub active_impl_bounds: verum_common::Set<(Text, Text)>,
    pub headers: Vec<super::source_protocol_header::SourceProtocolHeader>,
    pub projection_assumptions: Vec<crate::protocol::WhereClause>,
    pub completed: Map<Text, TypeDecl>,
    pub pending: Map<Text, TypeDecl>,
    pub depth: usize,
    pub failure: Option<Text>,
    pub aliases: Map<Text, (Option<Type>, Option<verum_common::List<Text>>)>,
    pub protocols: Map<Text, SourceProtocolRegistration>,
    pub variants: SourceVariantRegistration,
}

#[derive(Debug)]
pub(crate) struct SourceProtocolRegistration {
    pub publications: Vec<crate::protocol::ProtocolBindingReceipt>,
    pub unifier_name: bool,
    pub defaults: Option<(verum_common::Set<Text>, verum_common::Set<Text>)>,
}

#[derive(Debug, Default)]
pub(crate) struct SourceVariantRegistration {
    pub names: Map<Text, Option<Text>>,
    pub parents: Map<Text, Option<List<Text>>>,
    pub records: Map<Text, Option<indexmap::IndexMap<Text, Type>>>,
    pub paths: Map<Text, Option<List<crate::ty::PathConstructor>>>,
    pub arities: Map<Text, Option<usize>>,
    pub shared_names: Vec<(Text, Text)>,
    pub collisions: List<(Text, Text, Text)>,
}

fn restore_entries<T>(target: &mut Map<Text, T>, journal: Map<Text, Option<T>>) {
    for (name, previous) in journal {
        match previous {
            Some(value) => {
                target.insert(name, value);
            }
            None => {
                target.remove(&name);
            }
        }
    }
}

impl super::TypeChecker {
    /// All declaration entry points share the source-load transaction. A
    /// provisional recursive carrier is usable only until this whole load
    /// succeeds; a later field or dependency error rolls its namespace back.
    pub(super) fn with_source_type_registration<T>(
        &mut self,
        declaration: &TypeDecl,
        key: Text,
        register: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        if self
            .source_type_registration
            .pending
            .get(&key)
            .is_some_and(|pending| pending != declaration)
        {
            let message: Text =
                format!("conflicting source declarations while loading {key}").into();
            self.source_type_registration.failure = Some(message.clone());
            return Err(TypeError::UniverseBinding {
                message,
                span: declaration.span,
            });
        }
        let outermost = self.source_type_registration.depth == 0;
        if outermost {
            self.ctx.begin_type_definition_journal();
            self.unifier.begin_variant_registration_journal();
        }
        self.source_type_registration
            .pending
            .insert(key.clone(), declaration.clone());
        // Direct registration also publishes a local spelling. Keep the first
        // snapshot when a lazy load re-enters this same declaration.
        for name in [key.clone(), declaration.name.name.clone()] {
            self.source_type_registration
                .aliases
                .entry(name.clone())
                .or_insert_with(|| self.unifier.alias_binding(name.as_str()));
        }
        if matches!(
            declaration.body,
            verum_ast::TypeDeclBody::Protocol(_) | verum_ast::TypeDeclBody::Coinductive(_)
        ) {
            self.source_type_registration
                .protocols
                .entry(key.clone())
                .or_insert_with(|| SourceProtocolRegistration {
                    publications: Vec::new(),
                    unifier_name: self.unifier.protocol_name_is_registered(&key),
                    defaults: self.protocols_with_known_defaults.get(&key).cloned(),
                });
        }
        self.source_type_registration
            .variants
            .arities
            .entry(declaration.name.name.clone())
            .or_insert_with(|| {
                self.type_generics_count
                    .get(&declaration.name.name)
                    .copied()
            });
        self.source_type_registration.depth += 1;
        let result = register(self);
        self.source_type_registration.depth -= 1;
        if let Err(error) = &result {
            self.source_type_registration.failure = Some(error.to_string().into());
        }
        if outermost {
            let pending = std::mem::take(&mut self.source_type_registration.pending);
            let aliases = std::mem::take(&mut self.source_type_registration.aliases);
            let protocols = std::mem::take(&mut self.source_type_registration.protocols);
            if let Some(failure) = self.source_type_registration.failure.take() {
                self.ctx.finish_type_definition_journal(false);
                self.finish_source_variant_registrations(false);
                for (name, binding) in aliases {
                    self.unifier.restore_alias_binding(name, binding);
                }
                for (name, binding) in protocols {
                    for publication in binding.publications.into_iter().rev() {
                        self.protocol_checker
                            .write()
                            .restore_protocol_binding(publication);
                    }
                    self.unifier
                        .restore_protocol_name(name.clone(), binding.unifier_name);
                    match binding.defaults {
                        Some(defaults) => {
                            self.protocols_with_known_defaults.insert(name, defaults);
                        }
                        None => {
                            self.protocols_with_known_defaults.remove(&name);
                        }
                    }
                }
                for (key, declaration) in &pending {
                    // Pass one may precede this transaction. Only this exact
                    // declaration owns its restored forward placeholder. Keep
                    // previously accepted carriers and another owner's local
                    // placeholder; the journal already restored their aliases.
                    for name in [key, &declaration.name.name] {
                        if matches!(self.ctx.type_defs.get(name), Some(Type::Placeholder { name: owner, .. }) if owner == key)
                        {
                            self.ctx.type_defs.remove(name);
                        }
                    }
                }
                return match result {
                    Err(error) => Err(error),
                    Ok(_) => Err(TypeError::UniverseBinding {
                        message: format!("source declaration load for {key} failed: {failure}")
                            .into(),
                        span: declaration.span,
                    }),
                };
            }
            self.ctx.finish_type_definition_journal(true);
            self.finish_source_variant_registrations(true);
            for (key, declaration) in &pending {
                let (arity, alias) = self.completed_protocol_type_authority(key, declaration);
                self.protocol_checker.write().register_type_declaration_kind(
                    key.clone(), declaration.body.kind(), arity, alias,
                );
            }
            self.source_type_registration.completed.extend(pending);
        }
        result
    }

    /// Canonicalize only slots listed by the selected declaration. The same
    /// checked mixed-family carrier supplies role, domain and capture-safe
    /// substitution to protocol proofs; spelling is never an argument binder.
    fn completed_protocol_type_authority(
        &self, key: &Text, declaration: &TypeDecl,
    ) -> (Option<usize>, Option<crate::protocol::AssociatedTypeAssignment>) {
        use crate::context::GenericBinder;
        let Some(Type::Record(roster)) = self.ctx.lookup_type(&format!("__type_params_{key}")) else {
            return (None, None);
        };
        let arity = Some(roster.len());
        if !matches!(declaration.body, verum_ast::TypeDeclBody::Alias(_)) {
            return (arity, None);
        }
        let alias = (|| {
            let body = self.unifier.alias_binding(key).0?;
            let mut named = indexmap::IndexMap::new();
            let mut parameters = List::new();
            for (name, role) in roster {
                let role = self.substitute_declared_named_slots(role, &named).ok()?;
                let binder = match GenericBinder::declared(&role) {
                    Ok(binder) => binder,
                    Err(_) if matches!(&role, Type::Named { path, args }
                        if args.is_empty() && path.as_ident().is_some_and(|ident| ident.name == *name))
                        || matches!(&role, Type::Generic { name: own, args } if args.is_empty() && own == name) => {
                        let variable = crate::ty::TypeVar::fresh();
                        named.insert(name.clone(), Type::Var(variable));
                        GenericBinder::Type(variable)
                    }
                    _ => return None,
                };
                parameters.push(binder);
            }
            let body = self.substitute_declared_named_slots(&body, &named).ok()?;
            crate::protocol::AssociatedTypeAssignment::mixed_family(parameters, body).ok()
        })();
        // Failure retains Alias kind, so it cannot become nominal disjointness.
        (arity, alias)
    }

    pub(super) fn journal_variant_parent(&mut self, name: &Text) {
        if self.source_type_registration.depth != 0 {
            self.source_type_registration
                .variants
                .parents
                .entry(name.clone())
                .or_insert_with(|| self.variant_constructor_parents.get(name).cloned());
        }
    }

    pub(super) fn journal_variant_record(&mut self, name: &Text) {
        if self.source_type_registration.depth != 0 {
            self.source_type_registration
                .variants
                .records
                .entry(name.clone())
                .or_insert_with(|| self.variant_record_fields.get(name).cloned());
        }
    }

    pub(super) fn journal_variant_paths(&mut self, name: &Text) {
        if self.source_type_registration.depth != 0 {
            self.source_type_registration
                .variants
                .paths
                .entry(name.clone())
                .or_insert_with(|| self.hit_path_constructors.get(name).cloned());
        }
    }

    pub(super) fn finish_source_variant_registrations(&mut self, commit: bool) {
        self.unifier.finish_variant_registration_journal(commit);
        let journal = std::mem::take(&mut self.source_type_registration.variants);
        if commit {
            // The shared registry only receives complete declaration groups.
            // No failed group's provisional name can escape to another checker.
            let mut registry = self.protocol_checker.write();
            for (signature, name) in journal.shared_names {
                registry.register_variant_type_name(signature, name);
            }
            self.variant_collision_log.extend(journal.collisions);
        } else {
            restore_entries(&mut self.variant_type_names, journal.names);
            restore_entries(&mut self.variant_constructor_parents, journal.parents);
            restore_entries(&mut self.variant_record_fields, journal.records);
            restore_entries(&mut self.hit_path_constructors, journal.paths);
            restore_entries(&mut self.type_generics_count, journal.arities);
        }
    }
}
