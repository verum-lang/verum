//! Source declaration loads publish success only after recursive loading closes.
use crate::{Result, Type, TypeError};
use verum_ast::TypeDecl;
use verum_common::{List, Map, Shared, Text};
use verum_modules::ModuleDependencies;

#[derive(Debug, Default)]
pub(crate) struct SourceTypeRegistration {
    /// Lexical source declarations available while preparing one module.
    pub local_declarations: List<(Text, verum_common::Shared<verum_ast::Module>)>,
    /// Exact source methods index their published lookup locations. Body
    /// checking updates these locations without scanning unrelated methods.
    pub method_publications: Map<(Text, verum_ast::Span), List<(Text, Text)>>,
    /// Result declared by the current catch_unwind wrapper and the enclosing closure identity.
    pub(super) catch_unwind_body: Option<(Option<crate::aliasing::RefId>, Type)>,
    pub active_impl_bounds: verum_common::Set<(Text, Text)>,
    pub(super) loading_blankets: verum_common::Set<Text>,
    /// Public sequence methods imported by each source scope. Store the
    /// actual receiver pattern so a method on [Int] cannot apply to [Bool].
    pub(super) structural_methods: Map<Text, Vec<super::implementation_sources::ImportedStructuralMethod>>,
    pub headers: Vec<super::source_protocol_header::SourceProtocolHeader>,
    pub projection_assumptions: Vec<crate::protocol::WhereClause>,
    pub callable_assumptions: List<(Type, Type)>,
    /// Qualified constructor values staged with the declaration transaction.
    pub constructor_values: Map<Text, crate::context::TypeScheme>,
    completed: Map<Text, AcceptedSourceType>,
    pub pending: Map<Text, TypeDecl>,
    /// Exact nominal back-edges permitted by the active payload traversal.
    pub(super) forward_payloads: Map<Text, super::source_declarations::SourceTypeDeclaration>,
    /// Completed alias bodies usable by this transaction's own proof queries.
    /// Other checkers see them only after the entire registration commits.
    staged_aliases: Map<Text, crate::protocol::AssociatedTypeAssignment>,
    /// Fully elaborated nested declarations. Only proof queries issued by
    /// this transaction may use their kind and arity before publication.
    pub(super) staged_heads: crate::protocol::DeclarationHeads,
    pub depth: usize,
    pub failure: Option<Text>,
    pub aliases: Map<Text, (Option<Type>, Option<verum_common::List<Text>>)>,
    pub protocols: Map<Text, SourceProtocolRegistration>,
    pub variants: SourceVariantRegistration,
}

#[derive(Debug)]
struct AcceptedSourceType {
    declaration: TypeDecl,
    /// The actual file used when this type was prepared. Its imports remain
    /// available when a later pass checks the item outside the lexical frame.
    lexical_source: Option<Shared<verum_ast::Module>>,
    /// Recursive declarations commit together and share their checked inputs.
    dependencies: Shared<ModuleDependencies>,
}

impl SourceTypeRegistration {
    pub(super) fn proof_environment<'a>(
        &'a self, protocols: &'a [crate::protocol::WhereClause],
        callables: &'a Map<crate::ty::TypeVar, Type>,
    ) -> crate::protocol::ConstraintEnvironment<'a> {
        crate::protocol::ConstraintEnvironment {
            protocols, callables, local_heads: (!self.staged_heads.is_empty()).then_some(&self.staged_heads),
        }
    }

    pub(super) fn declaration(&self, key: &Text) -> Option<&TypeDecl> {
        self.pending.get(key).or_else(|| self.completed_declaration(key))
    }

    pub(super) fn completed_declaration(&self, key: &Text) -> Option<&TypeDecl> {
        self.completed.get(key).map(|accepted| &accepted.declaration)
    }

    pub(super) fn is_completed(&self, key: &Text) -> bool {
        self.completed.contains_key(key)
    }

    pub(super) fn lexical_source(
        &self, key: &Text, declaration: &TypeDecl,
    ) -> Option<Shared<verum_ast::Module>> {
        let accepted = self.completed.get(key)?;
        if accepted.declaration != *declaration {
            return None;
        }
        accepted.lexical_source.clone()
    }
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
    /// Existing derived fields, constructors and protocol publications belong
    /// to this checker. Rebinding one of their inputs requires a fresh checker.
    pub(super) fn require_source_dependencies(
        &mut self, key: &Text, span: verum_ast::Span,
    ) -> Result<()> {
        if let Some(accepted) = self.source_type_registration.completed.get(key)
            && !accepted.dependencies.reuse_in(&self.module_registry.read())
        {
            let message: Text = format!("source dependencies changed while loading {key}").into();
            if self.source_type_registration.depth != 0 {
                self.source_type_registration.failure = Some(message.clone());
            }
            return Err(TypeError::UniverseBinding { message, span });
        }
        Ok(())
    }

    /// The whole registration reads one immutable registry generation. Lazy
    /// writes still reach the live session registry, which is restored on exit.
    /// A changed observed input rolls back through the existing journals.
    fn observe_source_registration<T>(
        &mut self,
        declaration: &TypeDecl,
        key: &Text,
        register: impl FnOnce(&mut Self) -> Result<T>,
    ) -> (Result<T>, Shared<ModuleDependencies>) {
        let (snapshot, reads) = self.module_registry.read().observe_snapshot();
        let live = std::mem::replace(
            &mut self.module_registry,
            Shared::new(parking_lot::RwLock::new(snapshot)),
        );
        let result = register(self);
        self.module_registry = live;
        let dependencies = reads.finish();
        let result = result.and_then(|value| {
            dependencies.unchanged(&self.module_registry.read()).then_some(value).ok_or_else(|| TypeError::UniverseBinding {
                message: format!("source dependencies changed while loading {key}").into(),
                span: declaration.span,
            })
        });
        (result, Shared::new(dependencies))
    }

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
        self.require_source_dependencies(&key, declaration.span)?;
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
        // Eager preparation and lazy loads share the declaration's lexical
        // imports. A prior consumer may have populated the nominal cache
        // before this module reaches its ordinary two-pass registration.
        let register = |checker: &mut Self| {
            let owner = checker.current_module_path.clone();
            let has_source = key == Self::source_type_declaration_key(&owner, &declaration.name.name)
                && (checker.source_type_registration.local_declarations.iter()
                    .any(|(path, _)| path == &owner)
                    || checker.inline_modules.contains_key(&owner)
                    || checker.get_module_with_path_aliases(&owner, &checker.module_registry.read())
                        .is_some_and(|module| !module.ast.items.is_empty()));
            if has_source {
                checker.with_source_signature_scope(&owner, register)
            } else {
                register(checker)
            }
        };
        let (result, dependencies) = if outermost {
            let (result, reads) = self.observe_source_registration(declaration, &key, register);
            (result, Some(reads))
        } else {
            (register(self), None)
        };
        self.source_type_registration.depth -= 1;
        if let Err(error) = &result {
            self.source_type_registration.failure = Some(error.to_string().into());
        } else if !outermost {
            let (arity, alias) = self.completed_protocol_type_authority(&key, declaration);
            self.source_type_registration.staged_heads.insert(key.clone(), (declaration.body.kind(), arity));
            if let Some(alias) = alias {
                self.source_type_registration.staged_aliases.insert(key.clone(), alias);
            }
        }
        if outermost {
            self.source_type_registration.staged_heads.clear();
            self.source_type_registration.staged_aliases.clear();
            let pending = std::mem::take(&mut self.source_type_registration.pending);
            let aliases = std::mem::take(&mut self.source_type_registration.aliases);
            let constructors = std::mem::take(&mut self.source_type_registration.constructor_values);
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
                self.publish_context_protocol_binding(declaration, key, key.clone());
                // Loading a dependency does not mount its short name. Only a
                // surviving lexical type binding publishes that context name.
                if matches!(self.ctx.lookup_type(&declaration.name.name),
                    Some(Type::Named { path, .. }) if self.path_to_string(path) == *key)
                {
                    self.publish_context_protocol_binding(declaration, key, declaration.name.name.clone());
                }
            }
            // Lazy loads may finish inside a temporary value scope. Canonical
            // constructor identity survives it only after the entire load succeeds.
            for (name, constructor) in constructors {
                self.ctx.env.insert_root(name, constructor);
            }
            let dependencies = dependencies.expect("outermost registration observes its dependencies");
            let lexical = &self.source_type_registration.local_declarations;
            self.source_type_registration.completed.extend(pending.into_iter().map(|(key, declaration)| {
                let owner = key.rsplit_once('.').map_or("", |(path, _)| path);
                let lexical_source = lexical.iter().rev()
                    .find(|(path, _)| path.as_str() == owner)
                    .map(|(_, file)| file.clone());
                (key, AcceptedSourceType { declaration, lexical_source, dependencies: dependencies.clone() })
            }));
        }
        result
    }

    /// Normalize only a fully checked alias, preserving its predicate, binder roles
    /// and exact owner. Keep the named subject separately for implementation lookup.
    pub(super) fn staged_proof_receiver(&self, subject: &Type, span: verum_ast::Span)
        -> Result<Option<Type>>
    {
        use crate::ty::{Substitution, nominal_path_name};
        if self.source_type_registration.staged_aliases.is_empty() {
            return Ok(None);
        }
        let mut expanded = None;
        let mut visited = verum_common::Set::new();
        loop {
            let (key, arguments) = match expanded.as_ref().unwrap_or(subject) {
                Type::Named { path, args } => match nominal_path_name(path) {
                    Some(name) => (Text::from(name.as_ref()), args),
                    None => break,
                },
                Type::Generic { name, args } => (name.clone(), args),
                _ => break,
            };
            let Some(alias) = self.source_type_registration.staged_aliases.get(&key) else { break };
            if !visited.insert(key.clone()) {
                return Err(TypeError::OtherWithCodeSpanned { code: "E405".into(),
                    msg: format!("cyclic alias `{key}` in a protocol obligation").into(), span });
            }
            expanded = Some(alias.instantiate(arguments, &Substitution::new())
                .map_err(|msg| TypeError::OtherWithCodeSpanned { code: "E405".into(), msg, span })?);
        }
        Ok(expanded)
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
