//! A nominal application must establish the selected declaration's domain.
use super::TypeChecker;
use crate::{Result, Type, TypeError, protocol::WhereClause};
use verum_ast::{
    Span, TypeDecl,
    ty::{GenericParamKind, TypeBound, TypeBoundKind, TypeKind, WherePredicateKind},
};
use verum_common::{List, Map, Shared, Text};

// Hold the original immutable owner through lazy registration. Capturing all
// scopes first preserves their source snapshots without copying unrelated bodies.
enum BoundSource {
    Local(Shared<verum_ast::Module>),
    Registered(Shared<verum_modules::ModuleInfo>),
}

impl BoundSource {
    fn ast(&self) -> &verum_ast::Module {
        match self {
            Self::Local(module) => module,
            Self::Registered(module) => &module.ast,
        }
    }
}

fn refusal(span: Span, message: impl Into<Text>) -> TypeError {
    TypeError::OtherWithCodeSpanned {
        code: "E405".into(),
        msg: message.into(),
        span,
    }
}

#[derive(Default)]
pub(super) struct NominalPremises {
    protocols: List<WhereClause>,
    callables: List<(Type, Type)>,
}

impl TypeChecker {
    pub(super) fn extend_source_parameter_premises(
        &mut self,
        bounds: &Map<crate::ty::TypeVar, List<crate::protocol::ProtocolBound>>,
        types: &Map<crate::ty::TypeVar, List<Type>>,
    ) {
        self.source_type_registration
            .projection_assumptions
            .extend(bounds.iter().map(|(variable, bounds)| WhereClause {
                ty: Type::Var(*variable),
                bounds: bounds.clone(),
            }));
        self.source_type_registration
            .callable_assumptions
            .extend(types.iter().flat_map(|(variable, bounds)| {
                bounds
                    .iter()
                    .filter(|ty| matches!(ty, Type::Function { .. }))
                    .map(|bound| (Type::Var(*variable), bound.clone()))
            }));
    }

    /// Bounds belong to the declaration frame, including while its own fields
    /// and alias target are elaborated. A lazy registration cannot inherit a
    /// consumer's equally-spelled parameter assumptions.
    pub(super) fn begin_nominal_declaration_premises(
        &mut self,
        declaration: &TypeDecl,
    ) -> (NominalPremises, Result<()>) {
        let previous = NominalPremises {
            protocols: std::mem::take(&mut self.source_type_registration.projection_assumptions)
                .into_iter()
                .collect(),
            callables: std::mem::take(&mut self.source_type_registration.callable_assumptions),
        };
        let result = if matches!(
            declaration.body,
            verum_ast::TypeDeclBody::Protocol(_) | verum_ast::TypeDeclBody::Coinductive(_)
        ) {
            Ok(())
        } else {
            self.elaborate_nominal_bounds(declaration, true).map(|_| ())
        };
        (previous, result)
    }

    pub(super) fn finish_nominal_declaration_premises(&mut self, previous: NominalPremises) {
        self.source_type_registration.projection_assumptions =
            previous.protocols.into_iter().collect();
        self.source_type_registration.callable_assumptions = previous.callables;
    }

    fn elaborate_nominal_bounds(
        &mut self,
        declaration: &TypeDecl,
        assume: bool,
    ) -> Result<NominalPremises> {
        let mut clauses = NominalPremises::default();
        for parameter in &declaration.generics {
            let (name, bounds) = match &parameter.kind {
                GenericParamKind::Type { name, bounds, .. }
                | GenericParamKind::HigherKinded { name, bounds, .. }
                | GenericParamKind::KindAnnotated { name, bounds, .. } => (name, bounds),
                _ => continue,
            };
            if bounds.is_empty() {
                continue;
            }
            let subject = self.lookup_decl_param(&name.name).cloned().ok_or_else(|| {
                refusal(
                    parameter.span,
                    "nominal bound has no declaration-owned parameter",
                )
            })?;
            self.append_nominal_bounds(subject, bounds, assume, &mut clauses)?;
        }
        for source in [
            declaration.generic_where_clause.as_ref(),
            declaration.meta_where_clause.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            for predicate in &source.predicates {
                let WherePredicateKind::Type { ty, bounds } = &predicate.kind else {
                    return Err(refusal(
                        predicate.span,
                        "nominal predicate lacks a checked application obligation",
                    ));
                };
                let subject = self.ast_to_type(ty)?;
                self.append_nominal_bounds(subject, bounds, assume, &mut clauses)?;
            }
        }
        Ok(clauses)
    }

    fn append_nominal_bounds(
        &mut self,
        subject: Type,
        bounds: &[TypeBound],
        assume: bool,
        clauses: &mut NominalPremises,
    ) -> Result<()> {
        let mut protocols = List::new();
        for bound in bounds {
            match &bound.kind {
                TypeBoundKind::Equality(ty) | TypeBoundKind::GenericProtocol(ty)
                    if matches!(
                        ty.kind,
                        TypeKind::Function { .. } | TypeKind::Rank2Function { .. }
                    ) =>
                {
                    let requirement = self.ast_to_type(ty)?;
                    if assume {
                        self.source_type_registration
                            .callable_assumptions
                            .push((subject.clone(), requirement.clone()));
                    }
                    clauses.callables.push((subject.clone(), requirement));
                }
                _ => protocols.extend(self.elaborate_family_bounds(std::slice::from_ref(bound))?),
            }
        }
        if !protocols.is_empty() {
            let clause = WhereClause {
                ty: subject,
                bounds: protocols,
            };
            if assume {
                self.source_type_registration
                    .projection_assumptions
                    .push(clause.clone());
            }
            clauses.protocols.push(clause);
        }
        Ok(())
    }

    /// Called after the common argument telescope validates arity and roles,
    /// before alias erasure. The exact selected source key owns these bounds.
    pub(super) fn check_nominal_application_bounds(
        &mut self,
        key: &Text,
        arguments: &[Type],
        span: Span,
    ) -> Result<()> {
        let Some(declaration) = self.source_type_registration.declaration(key)
            .filter(|decl| {
                decl.has_application_bounds()
                    && !matches!(
                        decl.body,
                        verum_ast::TypeDeclBody::Protocol(_)
                            | verum_ast::TypeDeclBody::Coinductive(_)
                    )
            })
            .cloned()
        else {
            return Ok(());
        };
        let mut assumptions = self.source_type_registration.projection_assumptions.clone();
        assumptions.extend(
            self.type_var_bounds
                .iter()
                .filter(|(var, _)| self.unifier.is_rigid(**var))
                .map(|(var, bounds)| WhereClause {
                    ty: self.unifier.apply(&Type::Var(*var)),
                    bounds: bounds.clone(),
                }),
        );
        let parameters = match self.ctx.lookup_type(&format!("__type_params_{key}")) {
            Some(Type::Record(parameters)) => parameters,
            _ => {
                return Err(refusal(
                    span,
                    format!(
                        "bounded nominal application `{key}` lacks its declaration parameter roster"
                    ),
                ));
            }
        };
        if parameters.len() != arguments.len() {
            return Err(refusal(
                span,
                "nominal application does not match its declaration telescope",
            ));
        }
        let frame = parameters
            .keys()
            .cloned()
            .zip(arguments.iter().cloned())
            .collect();
        let owner = key
            .rsplit_once('.')
            .map(|(owner, _)| owner)
            .unwrap_or("cog");
        let previous_owner = std::mem::replace(&mut self.current_module_path, owner.into());
        let previous_source = std::mem::replace(&mut self.resolving_source_signature, true);
        self.push_decl_param_frame(frame);
        let requirements = self.elaborate_nominal_bounds(&declaration, false);
        self.pop_decl_param_frame();
        self.resolving_source_signature = previous_source;
        self.current_module_path = previous_owner;
        let requirements = requirements?;
        if requirements.protocols.is_empty() && requirements.callables.is_empty() {
            return Ok(());
        }
        // Named slots are an older source template representation. Only the
        // current declaration roster authorizes turning them into rigid proof
        // variables; supplied nominal names never acquire binder identity.
        let mut named = indexmap::IndexMap::new();
        if let Some(frame) = self.decl_param_frames.last() {
            for (name, ty) in frame {
                if matches!(ty, Type::Named { path, args } if args.is_empty() && path.as_ident().is_some_and(|id| &id.name == name))
                {
                    named.insert(name.clone(), Type::Var(crate::ty::TypeVar::fresh()));
                }
            }
        }
        for (actual, required) in requirements.callables {
            let actual = if matches!(actual, Type::Function { .. }) {
                actual
            } else {
                self.source_type_registration
                    .callable_assumptions
                    .iter()
                    .find(|(subject, _)| subject == &actual)
                    .map(|(_, ty)| ty.clone())
                    .ok_or_else(|| {
                        refusal(
                            span,
                            "nominal callable argument lacks its declared contract",
                        )
                    })?
            };
            let actual = self
                .substitute_declared_named_slots(&actual, &named)
                .map_err(|message| refusal(span, message))?;
            let required = self
                .substitute_declared_named_slots(&required, &named)
                .map_err(|message| refusal(span, message))?;
            let rigid = self.unifier.rigid_snapshot();
            self.unifier.enter_rigid_scope(
                actual
                    .free_vars()
                    .into_iter()
                    .chain(required.free_vars())
                    .map(|var| (var, Text::from("nominal argument"), false, None)),
            );
            let saved = self.unifier.save_substitution();
            let result = self.check_call_type_bound(&actual, &required, span, 0);
            self.unifier.restore_substitution(saved);
            self.unifier.exit_rigid_scope(rigid);
            result?;
        }
        let normalize = |ty: &Type| {
            self.substitute_declared_named_slots(ty, &named)
                .map_err(|message| refusal(span, message))
        };
        let mut callables = Map::new();
        for (subject, contract) in &self.source_type_registration.callable_assumptions {
            if let Type::Var(variable) = normalize(subject)? {
                let contract = normalize(contract)?;
                if callables
                    .get(&variable)
                    .is_some_and(|previous| previous != &contract)
                {
                    return Err(refusal(
                        span,
                        "nominal callable premises require a represented intersection",
                    ));
                }
                callables.insert(variable, contract);
            }
        }
        let normalize_bound = |bound: &crate::protocol::ProtocolBound| -> Result<_> {
            let mut bound = bound.clone();
            bound.args = bound.args.iter().map(normalize).collect::<Result<_>>()?;
            bound.associated_types = bound
                .associated_types
                .iter()
                .map(|(name, ty)| Ok((name.clone(), normalize(ty)?)))
                .collect::<Result<_>>()?;
            Ok(bound)
        };
        let assumptions: List<_> = assumptions
            .iter()
            .map(|clause| {
                Ok(WhereClause {
                    ty: normalize(&clause.ty)?,
                    bounds: clause
                        .bounds
                        .iter()
                        .map(normalize_bound)
                        .collect::<Result<_>>()?,
                })
            })
            .collect::<Result<_>>()?;
        let requirements: List<_> = requirements
            .protocols
            .iter()
            .map(|clause| {
                Ok((
                    clause.clone(),
                    WhereClause {
                        ty: normalize(&clause.ty)?,
                        bounds: clause
                            .bounds
                            .iter()
                            .map(normalize_bound)
                            .collect::<Result<_>>()?,
                    },
                ))
            })
            .collect::<Result<_>>()?;
        for (source, requirement) in requirements {
            let mut remaining = List::new();
            for (source_bound, bound) in source.bounds.iter().zip(requirement.bounds) {
                if !self
                    .protocol_checker
                    .read()
                    .bound_follows_from_assumptions(&requirement.ty, &bound, &assumptions)
                    .map_err(|error| refusal(span, error))?
                {
                    self.load_nominal_bound_source(&source.ty, source_bound)?;
                    remaining.push(bound);
                }
            }
            self.prove_associated_bounds(
                &requirement.ty,
                &remaining,
                &assumptions,
                &callables,
                span,
            )?;
        }
        Ok(())
    }

    fn load_nominal_bound_source(
        &mut self,
        subject: &Type,
        bound: &crate::protocol::ProtocolBound,
    ) -> Result<()> {
        let subject_name = self.get_type_name(subject);
        let protocol = self.path_to_string(&bound.protocol);
        let protocol_owner = protocol
            .rsplit_once('.')
            .map(|(owner, _)| owner)
            .unwrap_or("cog");
        let mut scopes = List::new();
        for owner in [self.current_module_path.as_str(), protocol_owner] {
            if scopes
                .iter()
                .any(|(prior, _): &(Text, BoundSource)| prior == owner)
            {
                continue;
            }
            if let Some((_, module)) = self
                .source_type_registration
                .local_declarations
                .iter()
                .rev()
                .find(|(path, _)| path == owner)
            {
                scopes.push((Text::from(owner), BoundSource::Local(module.clone())));
            } else if let Some(module) = self.module_registry.read().get_by_path(owner) {
                scopes.push((Text::from(owner), BoundSource::Registered(module)));
            }
        }
        for (owner, source) in scopes {
            let module = source.ast();
            let frames = std::mem::take(&mut self.decl_param_frames);
            let premises =
                std::mem::take(&mut self.source_type_registration.projection_assumptions);
            let callables = std::mem::take(&mut self.source_type_registration.callable_assumptions);
            let previous_owner = std::mem::replace(&mut self.current_module_path, owner.clone());
            let result: Result<()> = (|| {
                // Blanket candidates have no nominal receiver owner. Their
                // actual source telescope, not a guessed concrete receiver,
                // determines whether they apply to this proof obligation.
                for item in &module.items {
                    if !self.cfg_evaluator.should_include(&item.attributes) {
                        continue;
                    }
                    if let verum_ast::ItemKind::Impl(implementation) = &item.kind
                        && let verum_ast::decl::ImplKind::Protocol {
                            protocol: path,
                            for_type,
                            ..
                        } = &implementation.kind
                        && self.is_blanket_impl_for_type(for_type, &implementation.generics)
                    {
                        let selected = self.resolve_protocol_reference(path)?;
                        if self.path_to_string(&selected) == protocol {
                            self.register_blanket_impl_from_ast(implementation)?;
                        }
                    }
                }
                if let Some(name) = &subject_name {
                    let local = name
                        .as_str()
                        .strip_prefix(&format!("{owner}."))
                        .unwrap_or(name.as_str());
                    self.import_impl_blocks_for_type_in_module_selected(
                        module,
                        local,
                        &owner,
                        Some(&protocol),
                    )?;
                }
                Ok(())
            })();
            self.current_module_path = previous_owner;
            self.source_type_registration.callable_assumptions = callables;
            self.source_type_registration.projection_assumptions = premises;
            self.decl_param_frames = frames;
            result?;
        }
        Ok(())
    }
}
