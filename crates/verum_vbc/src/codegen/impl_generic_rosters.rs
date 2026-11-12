//! Source declaration headers available before layout and method collection.
//! A binder roster is a fact about one selected nominal declaration. It neither
//! invents a record layout nor imports a same-spelled foreign type's parameters.
use std::borrow::Cow;

use super::{CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen};
use crate::types::{ProtocolId, StringId, TypeId, TypeParamDescriptor, TypeParamId, TypeRef};
use verum_ast::{FunctionDecl, Type, TypeDecl, TypeKind, decl::ImplKind, ty::GenericParamKind};

#[derive(Debug, Clone)]
pub(super) struct DeclaredGenericParameter {
    pub(super) name: String,
    /// Runtime representation resolved in this declaration's own lexical scope.
    /// None does not erase the source value role or authorize another carrier.
    pub(super) value_carrier: Option<TypeRef>,
    pub(super) witness_slot: Option<TypeParamId>,
    /// Explicit source position for callable-local parameters only.
    pub(super) explicit_slot: Option<usize>,
    /// The source declaration establishes kind, defaults and refinements before
    /// a nominal layout or semantic descriptor is complete.
    pub(super) source: Option<std::sync::Arc<verum_ast::ty::GenericParam>>,
}

impl DeclaredGenericParameter {
    pub(super) fn value_parameter(&self) -> Option<&verum_ast::ty::GenericParam> {
        self.source.as_deref().filter(|parameter| {
            matches!(
                parameter.kind,
                GenericParamKind::Meta { .. } | GenericParamKind::Const { .. }
            )
        })
    }
}

pub(super) type GenericRoster = Vec<DeclaredGenericParameter>;

/// The declaring identity exists before an executable layout or descriptor.
/// Keep it with the roster so every consumer uses the same source authority.
#[derive(Debug, Clone)]
pub(super) struct SourceGenericRoster {
    identity: verum_common::Text,
    parameters: Option<GenericRoster>,
}

pub(super) struct DeclaredGenericParameters {
    pub(super) scope: std::collections::HashMap<String, u16>,
    pub(super) parameters: smallvec::SmallVec<[TypeParamDescriptor; 2]>,
}

fn validate_nominal_default(
    ty: &Type,
    preceding: &[DeclaredGenericParameter],
    has_receiver: bool,
) -> CodegenResult<()> {
    use verum_ast::visitor::{Visitor, walk_expr, walk_type};
    struct DefaultScope<'a> {
        preceding: &'a [DeclaredGenericParameter],
        has_receiver: bool,
        error: Option<verum_ast::Span>,
        depth: usize,
        budget: usize,
    }
    impl DefaultScope<'_> {
        fn enter(&mut self, span: verum_ast::Span) -> bool {
            if self.depth >= 128 || self.budget == 0 {
                self.error = Some(span);
            }
            if self.error.is_some() {
                return false;
            }
            self.depth += 1;
            self.budget -= 1;
            true
        }
    }
    impl Visitor for DefaultScope<'_> {
        fn visit_type(&mut self, ty: &Type) {
            if matches!(ty.kind, TypeKind::Refined { .. }) {
                self.error = Some(ty.span);
            }
            if !self.enter(ty.span) {
                return;
            }
            walk_type(self, ty);
            self.depth -= 1;
        }
        fn visit_expr(&mut self, expression: &verum_ast::Expr) {
            if !self.enter(expression.span) {
                return;
            }
            walk_expr(self, expression);
            self.depth -= 1;
        }
        fn visit_path(&mut self, path: &verum_ast::Path) {
            if path.as_ident().is_some_and(|name| {
                self.preceding.iter().any(|parameter| {
                    parameter.name == name.name.as_str() && parameter.value_parameter().is_some()
                })
            }) || (!self.has_receiver && path.segments.iter()
                .any(|part| matches!(part, verum_ast::ty::PathSegment::SelfValue)))
            {
                self.error = Some(path.span);
            }
        }
    }
    let mut scope = DefaultScope {
        preceding,
        has_receiver,
        error: None,
        depth: 0,
        budget: 4096,
    };
    scope.visit_type(ty);
    scope.error.map_or(Ok(()), |span| Err(CodegenError::with_span(
        CodegenErrorKind::InvalidSourceSignature("nominal default requires unsupported dependent value, refinement, or recursive receiver evidence".into()), span)))
}

impl VbcCodegen {
    /// Complete nominal applications before their argument vector becomes a
    /// semantic witness. Defaults belong to the selected declaration and see
    /// only its preceding binders, never unrelated caller names.
    pub(super) fn complete_nominal_arguments(
        &self,
        nominal: TypeId,
        mut arguments: Vec<TypeRef>,
        span: verum_ast::Span,
        active_defaults: &[TypeId],
        receiver: Option<&TypeRef>,
    ) -> CodegenResult<Vec<TypeRef>> {
        let Some(roster) = self.declaration_generic_roster(nominal) else {
            // This mapper does not synthesize a header for opaque imported
            // carriers. A consumer needing source authority rejects that case.
            return Ok(arguments);
        };
        let identity = self.source_generic_rosters.get(&nominal)
            .map(|header| Cow::Borrowed(&header.identity))
            .or_else(|| self.type_by_id(nominal)
                .and_then(|descriptor| self.type_declaration_identity(descriptor))
                .map(Cow::Owned));
        let invalid = |message: String| {
            let mut error =
                CodegenError::with_span(CodegenErrorKind::InvalidSourceSignature(message), span);
            error.context = identity
                .as_ref()
                .map(|name| format!("nominal application of '{name}'"));
            error
        };
        if arguments.len() > roster.len() {
            return Err(invalid(format!(
                "nominal application has {} arguments for {} declared parameters",
                arguments.len(),
                roster.len()
            )));
        }
        if arguments.len() == roster.len() {
            return Ok(arguments);
        }
        if active_defaults.contains(&nominal) || active_defaults.len() >= 128 {
            return Err(invalid(
                "cyclic or excessively deep nominal default expansion".into(),
            ));
        }
        let mut nested_defaults = active_defaults.to_vec();
        nested_defaults.push(nominal);
        let owner = identity.as_ref()
            .and_then(|name| name.rsplit_once('.').map(|(owner, _)| owner.to_owned()))
            .ok_or_else(|| invalid("nominal defaults lack their declaration owner".into()))?;
        let mut substitution = super::source_substitution::SourceTypeSubstitution::default();
        for (index, parameter) in roster.iter().enumerate() {
            if index == arguments.len() {
                let default = parameter
                    .source
                    .as_deref()
                    .and_then(|source| match &source.kind {
                        GenericParamKind::Type { default, .. } => default.as_ref(),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        invalid(format!(
                            "nominal application is missing required argument '{}'",
                            parameter.name
                        ))
                    })?;
                validate_nominal_default(default, &roster[..index], receiver.is_some())?;
                let mut complete = true;
                let mut active = verum_common::Set::new();
                let mut cache = verum_common::Map::new();
                let mut count = |expression: &verum_ast::Expr| {
                    let value = self
                        .const_eval_i64_in_scope(
                            expression,
                            Some(&owner),
                            None,
                            false,
                            &mut active,
                            &mut cache,
                            0,
                        )
                        .ok()
                        .flatten()
                        .and_then(|n| u64::try_from(n).ok());
                    complete &= value.is_some();
                    value
                };
                let argument = substitution.elaborate(&Default::default(), |scope, values| {
                    self.resolve_type_ref_in_scope(
                        default,
                        scope,
                        true,
                        Some(&owner),
                        &mut count,
                        receiver,
                        values,
                        &nested_defaults,
                    )
                })?;
                if !complete {
                    return Err(invalid(
                        "nominal default has an unresolved array extent".into(),
                    ));
                }
                arguments.push(argument);
            }
            // Value-dependent defaults need scalar-kind/obligation elaboration;
            // validate_nominal_default refuses them rather than erasing evidence.
            if parameter.value_parameter().is_none() {
                substitution.insert(
                    parameter.name.clone(),
                    super::ImplReceiverWitness {
                        ty: arguments[index].clone(),
                        semantic: false,
                    },
                );
            }
        }
        Ok(arguments)
    }

    /// Every executable descriptor uses the same dense slots as its claimed
    /// source header. Lifetimes have no runtime slot; value parameters do.
    pub(super) fn declared_generic_parameters(
        &mut self,
        id: TypeId,
        declaration: &TypeDecl,
    ) -> CodegenResult<DeclaredGenericParameters> {
        let invalid = |message: String| {
            CodegenError::with_span(
                CodegenErrorKind::InvalidSourceSignature(message),
                declaration.span,
            )
        };
        let names: Vec<_> = self
            .declaration_generic_roster(id)
            .ok_or_else(|| {
                invalid(format!(
                    "type '{}' has an unsupported generic parameter roster",
                    declaration.name.name
                ))
            })?
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect();
        let mut scope = std::collections::HashMap::new();
        for (index, name) in names.iter().enumerate() {
            let index = u16::try_from(index).map_err(|_| {
                invalid(format!(
                    "type '{}' exceeds the VBC generic index space",
                    declaration.name.name
                ))
            })?;
            if scope.insert(name.clone(), index).is_some() {
                return Err(invalid(format!(
                    "type '{}' repeats generic parameter '{name}'",
                    declaration.name.name
                )));
            }
        }
        let mut parameters = smallvec::SmallVec::new();
        for generic in &declaration.generics {
            let (name, bounds, default) = match &generic.kind {
                GenericParamKind::Lifetime { .. } => continue,
                GenericParamKind::Type {
                    name,
                    bounds,
                    default,
                } => (name, bounds.as_slice(), default.as_ref()),
                GenericParamKind::HigherKinded { name, bounds, .. } => {
                    (name, bounds.as_slice(), None)
                }
                GenericParamKind::Const { name, .. } | GenericParamKind::Meta { name, .. } => {
                    (name, &[][..], None)
                }
                _ => {
                    return Err(invalid(format!(
                        "type '{}' has an unsupported generic parameter",
                        declaration.name.name
                    )));
                }
            };
            let index = *scope.get(name.name.as_str()).ok_or_else(|| {
                invalid(format!(
                    "generic parameter '{}' has no claimed slot",
                    name.name
                ))
            })?;
            if names.get(parameters.len()).map(String::as_str) != Some(name.name.as_str()) {
                return Err(invalid(format!(
                    "type '{}' disagrees with its claimed generic roster",
                    declaration.name.name
                )));
            }
            // A protocol default may depend on the receiver of a future
            // implementation. Its source template is authoritative until that
            // receiver is selected; no runtime TypeRef can represent it here.
            let receiver_dependent = default.is_some_and(|ty| {
                use verum_ast::visitor::{Visitor, walk_type};
                struct Receiver(bool);
                impl Visitor for Receiver {
                    fn visit_type(&mut self, ty: &Type) {
                        if VbcCodegen::ast_type_is_bare_self(ty) {
                            self.0 = true;
                        } else {
                            walk_type(self, ty);
                        }
                    }
                }
                let mut receiver = Receiver(false);
                receiver.visit_type(ty);
                receiver.0
            });
            let deferred = declaration.body.is_protocol_like() && receiver_dependent;
            if let Some(default) = default.filter(|_| declaration.body.is_protocol_like() && !deferred)
                && declaration.generics.iter().any(|formal| matches!(
                    formal.kind, GenericParamKind::Const { .. } | GenericParamKind::Meta { .. }
                ))
            {
                self.reject_protocol_value_as_type(default, &declaration.generics, &scope)
                    .map_err(|error| error.with_context(format!("default of protocol '{}', parameter '{}'",
                        crate::module::qualify_module_name(self.lexical_type_scope(), declaration.name.name.as_str()),
                        name.name)))?;
            }
            let mut descriptor = TypeParamDescriptor {
                name: StringId(self.ctx.intern_string_raw(name.name.as_str())),
                id: TypeParamId(index),
                default: default
                    .filter(|_| !deferred)
                    .map(|ty| self.resolve_signature_type_ref(ty, &scope))
                    .transpose()?,
                ..Default::default()
            };
            for bound in bounds {
                match &bound.kind {
                    verum_ast::ty::TypeBoundKind::Protocol(path) => {
                        let name = Self::format_type_path(path);
                        let id = self.nominal_type_id(&name).ok_or_else(|| {
                            invalid(format!(
                                "unresolved protocol bound '{name}' on '{}'",
                                declaration.name.name
                            ))
                        })?;
                        descriptor.bounds.push(ProtocolId(id.0));
                    }
                    verum_ast::ty::TypeBoundKind::Equality(ty)
                        if matches!(ty.kind, TypeKind::Function { .. }) =>
                    {
                        descriptor
                            .type_bounds
                            .push(self.resolve_signature_type_ref(ty, &scope)?);
                    }
                    _ => {}
                }
            }
            parameters.push(descriptor);
        }
        Ok(DeclaredGenericParameters { scope, parameters })
    }

    pub(super) fn claim_generic_roster(&mut self, id: TypeId, declaration: &TypeDecl, identity: &str) {
        self.source_generic_rosters.insert(id, SourceGenericRoster {
            identity: identity.into(),
            parameters: Self::source_generic_roster(declaration),
        });
    }

    fn source_generic_roster(declaration: &TypeDecl) -> Option<GenericRoster> {
        Self::generic_roster(&declaration.generics)
    }

    pub(super) fn generic_roster(parameters: &[verum_ast::ty::GenericParam]) -> Option<GenericRoster> {
        parameters.iter()
            .filter(|parameter| !matches!(parameter.kind, GenericParamKind::Lifetime { .. }))
            .map(|parameter| {
                let name = match &parameter.kind {
                    GenericParamKind::Type { name, .. }
                    | GenericParamKind::HigherKinded { name, .. } => name,
                    GenericParamKind::Const { name, .. } | GenericParamKind::Meta { name, .. } => {
                        name
                    }
                    _ => return None,
                };
                Some(DeclaredGenericParameter {
                    value_carrier: None,
                    witness_slot: None,
                    explicit_slot: None,
                    name: name.name.to_string(),
                    source: Some(std::sync::Arc::new(parameter.clone())),
                })
            })
            .collect()
    }

    /// A known unsupported source header remains unknown. It cannot fall back
    /// to an erased descriptor and accidentally reinterpret value/level kinds.
    pub(super) fn declaration_generic_roster(
        &self,
        id: TypeId,
    ) -> Option<Cow<'_, [DeclaredGenericParameter]>> {
        if let Some(roster) = self.source_generic_rosters.get(&id) {
            return roster.parameters
                .as_ref()
                .map(|roster| Cow::Borrowed(roster.as_slice()));
        }
        let descriptor = self.type_by_id(id)?;
        if let Some(source) = &descriptor.source_declaration {
            return Self::source_generic_roster(source).map(Cow::Owned);
        }
        // Imported executable descriptors declare only their actual type slots.
        // No absent source value parameter is reconstructed from a name.
        descriptor
            .type_params
            .iter()
            .map(|parameter| {
                Some(DeclaredGenericParameter {
                    value_carrier: None,
                    witness_slot: None,
                    explicit_slot: None,
                    name: self.ctx.strings.get(parameter.name.0 as usize)?.clone(),
                    source: None,
                })
            })
            .collect::<Option<Vec<_>>>()
            .map(Cow::Owned)
    }

    pub(super) fn implementation_nominal_id(&self, implementation: &ImplKind) -> Option<TypeId> {
        let target = match implementation {
            ImplKind::Inherent(target)
            | ImplKind::Protocol {
                for_type: target, ..
            } => target,
        };
        self.implementation_type_nominal_id(target)
    }

    pub(super) fn implementation_type_nominal_id(&self, target: &Type) -> Option<TypeId> {
        match &target.kind {
            TypeKind::Generic { base, .. }
            | TypeKind::Reference { inner: base, .. }
            | TypeKind::CheckedReference { inner: base, .. }
            | TypeKind::UnsafeReference { inner: base, .. }
            | TypeKind::Pointer { inner: base, .. } => self.implementation_type_nominal_id(base),
            TypeKind::Path(path) => self.nominal_type_id(&Self::format_type_path(path)),
            _ => self
                .extract_impl_type_name_from_type(target)
                .and_then(|name| self.nominal_type_id(&name)),
        }
    }

    /// Entering an implementation selects its own declaration before any Self
    /// witness is installed. An enclosing method's carried receiver cannot
    /// supply the new implementation's parameter order.
    pub(super) fn declared_impl_generic_param_maps(
        &self,
        function: &FunctionDecl,
        implementation: &ImplKind,
        generics: &[String],
    ) -> (
        std::collections::HashMap<String, u16>,
        std::collections::HashMap<String, u16>,
    ) {
        Self::extend_function_generic_scope(
            function,
            self.declared_impl_generic_scope(implementation, generics),
        )
    }

    /// The implementation prefix is independent of any method-local binder.
    /// Retaining it lets deferred methods share one exact receiver witness.
    pub(super) fn declared_impl_generic_scope(
        &self,
        implementation: &ImplKind,
        generics: &[String],
    ) -> std::collections::HashMap<String, u16> {
        let parent_names = self
            .implementation_nominal_id(implementation)
            .and_then(|id| self.declaration_generic_roster(id))
            .map(|roster| {
                roster
                    .iter()
                    .map(|parameter| parameter.name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self::implementation_generic_scope(&parent_names, generics)
    }

    /// Exact carried Self takes precedence; otherwise source lexical lookup
    /// selects the declaration. A missing binding never consults a global leaf.
    pub(super) fn method_parent_generic_names(&self, name: &str) -> Option<Vec<String>> {
        let id = self
            .current_impl_receiver
            .as_ref()
            .and_then(|receiver| match &receiver.ty {
                TypeRef::Concrete(id) | TypeRef::Instantiated { base: id, .. } => Some(*id),
                _ => None,
            })
            .or_else(|| self.nominal_type_id(name))?;
        Some(
            self.declaration_generic_roster(id)?
                .iter()
                .map(|parameter| parameter.name.clone())
                .collect(),
        )
    }
}
