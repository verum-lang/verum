//! Declaration-owned protocol application arguments.
//!
//! Positional kinds come from the retained source telescope. A meta value is
//! carried as a value, never replaced with the runtime type of its expression.

use super::TypeChecker;
use crate::{Result, Type, TypeError};
use indexmap::IndexMap;
use verum_ast::ty::GenericParamKind;
use verum_ast::{GenericArg, GenericParam, Path, Span};
use verum_common::{ConstValue, List, Text};

fn invalid(span: Span, message: impl Into<Text>) -> TypeError {
    TypeError::OtherWithCodeSpanned {
        code: "E405".into(),
        msg: message.into(),
        span,
    }
}

impl TypeChecker {
    /// One source representation for value binders in declarations, lazy
    /// imports and method/function scopes. A value binder is never a TypeVar.
    pub(super) fn source_value_parameter(&mut self, parameter: &GenericParam) -> Result<Type> {
        let (name, ty, refinement) = match &parameter.kind {
            GenericParamKind::Meta {
                name,
                ty,
                refinement,
            } => (name, ty, refinement.as_ref()),
            GenericParamKind::Const { name, ty } => (name, ty, None),
            _ => {
                return Err(invalid(
                    parameter.span,
                    "expected a declared value parameter",
                ));
            }
        };
        let base = self.ast_to_type(ty)?;
        let refinement = refinement.map(|predicate| crate::refinement::RefinementPredicate {
            predicate: (**predicate).clone(),
            binding: crate::refinement::RefinementBinding::Sigma(name.name.clone()),
            span: ty.span,
            provenance: crate::refinement::PredicateProvenance::Declared,
        });
        Ok(Type::meta(name.name.clone(), base, refinement))
    }

    pub(super) fn protocol_application_arguments(
        &mut self,
        protocol: &Path,
        arguments: &[GenericArg],
        receiver: Option<&Type>,
    ) -> Result<List<Type>> {
        // Callers have already selected this canonical declaration in their
        // lexical scope. Re-resolving its owned identity as a new source name
        // would re-enter lazy registration inside an implementation scope.
        let identity = self.path_to_string(protocol);
        let (parameters, arity) = {
            let checker = self.protocol_checker.read();
            let declaration = checker
                .get_protocol_definition(identity.as_str())
                .ok_or_else(|| {
                    invalid(
                        protocol.span,
                        format!("protocol `{identity}` has no declaration"),
                    )
                })?;
            (
                declaration.source_parameters.clone(),
                declaration.type_params.len(),
            )
        };
        let Some(parameters) = parameters else {
            if arity == 0 && arguments.is_empty() {
                return Ok(List::new());
            }
            return Err(invalid(
                protocol.span,
                format!("protocol `{identity}` lacks its declaration-owned parameter header"),
            ));
        };
        let owner = identity
            .as_str()
            .rsplit_once('.')
            .map_or("", |(owner, _)| owner);
        self.elaborate_declared_arguments(&parameters, owner, arguments, receiver, protocol.span)
            .map_err(|error| {
                invalid(
                    error.span(),
                    format!("protocol application `{identity}`: {error}"),
                )
            })
    }

    /// Elaborate supplied arguments in their caller's scope, and defaults and
    /// carrier annotations in the declaration's owner with preceding binders.
    pub(super) fn elaborate_declared_arguments(
        &mut self,
        parameters: &[GenericParam],
        owner: &str,
        arguments: &[GenericArg],
        receiver: Option<&Type>,
        span: Span,
    ) -> Result<List<Type>> {
        if arguments
            .iter()
            .any(|arg| matches!(arg, GenericArg::Lifetime(_) | GenericArg::Binding(_)))
        {
            return Err(invalid(
                span,
                "lifetime or named-equality arguments need their own checked application evidence",
            ));
        }
        let mut arguments = arguments.iter();
        let mut bindings = IndexMap::new();
        let mut result = List::new();
        for parameter in parameters {
            if matches!(parameter.kind, GenericParamKind::Lifetime { .. }) {
                return Err(invalid(
                    parameter.span,
                    "lifetime parameters have no represented application evidence",
                ));
            }
            let argument = arguments.next();
            let (name, resolved) = match &parameter.kind {
                GenericParamKind::Type { name, default, .. } => {
                    let resolved = match argument {
                        Some(argument) => {
                            let annotation =
                                argument.type_for_declared_slot().ok_or_else(|| {
                                    invalid(span, "a type parameter requires a type argument")
                                })?;
                            self.checked_protocol_type_argument(&annotation)?
                        }
                        None => {
                            let default = default.as_ref().ok_or_else(|| {
                                invalid(span, format!("missing type argument for `{}`", name.name))
                            })?;
                            self.in_argument_declaration_scope(
                                owner,
                                &bindings,
                                receiver,
                                |checker| checker.checked_protocol_type_argument(default),
                            )?
                        }
                    };
                    (&name.name, resolved)
                }
                GenericParamKind::Meta {
                    name,
                    ty,
                    refinement,
                    ..
                } => {
                    if refinement.is_some() {
                        return Err(invalid(
                            parameter.span,
                            "refined protocol values require a checked refinement witness",
                        ));
                    }
                    let carrier = self.in_argument_declaration_scope(
                        owner,
                        &bindings,
                        receiver,
                        |checker| checker.ast_to_type(ty),
                    )?;
                    let carrier = self.unifier.normalize_aliases(&carrier);
                    let argument = argument.ok_or_else(|| {
                        invalid(span, format!("missing value argument for `{}`", name.name))
                    })?;
                    (
                        &name.name,
                        self.checked_protocol_value_argument(argument, &carrier, span)?,
                    )
                }
                GenericParamKind::Const { name, ty } => {
                    let carrier = self.in_argument_declaration_scope(
                        owner,
                        &bindings,
                        receiver,
                        |checker| checker.ast_to_type(ty),
                    )?;
                    let carrier = self.unifier.normalize_aliases(&carrier);
                    let argument = argument.ok_or_else(|| {
                        invalid(span, format!("missing value argument for `{}`", name.name))
                    })?;
                    (
                        &name.name,
                        self.checked_protocol_value_argument(argument, &carrier, span)?,
                    )
                }
                _ => {
                    return Err(invalid(
                        parameter.span,
                        "protocol parameter kind has no exact application argument carrier",
                    ));
                }
            };
            bindings.insert(name.clone(), resolved.clone());
            result.push(resolved);
        }
        if arguments.next().is_some() {
            return Err(invalid(span, "too many positional protocol arguments"));
        }
        Ok(result)
    }

    fn in_argument_declaration_scope<T>(
        &mut self,
        owner: &str,
        bindings: &IndexMap<Text, Type>,
        receiver: Option<&Type>,
        action: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let old_owner = self.current_module_path.clone();
        let old_source = self.resolving_source_signature;
        let old_self = self.current_self_type.clone();
        self.set_current_module_path(owner);
        self.resolving_source_signature = true;
        self.set_current_self_type(receiver.cloned());
        self.push_decl_param_frame(bindings.clone());
        let result = action(self);
        self.pop_decl_param_frame();
        self.set_current_self_type(old_self);
        self.set_current_module_path(old_owner);
        self.resolving_source_signature = old_source;
        result
    }

    fn checked_protocol_type_argument(&mut self, annotation: &verum_ast::Type) -> Result<Type> {
        use verum_ast::TypeKind;
        let _depth =
            super::ThreadLocalDepthGuard::new(&super::AST_TO_TYPE_DEPTH, 64).ok_or_else(|| {
                invalid(
                    annotation.span,
                    "declared argument elaboration exceeds its structural limit",
                )
            })?;
        let resolved = match &annotation.kind {
            TypeKind::Generic { base, args } => {
                let verum_ast::TypeKind::Path(path) = &base.kind else {
                    return Err(invalid(
                        base.span,
                        "an applied type argument needs its declaration-owned constructor header",
                    ));
                };
                let (constructor, key) =
                    self.resolve_qualified_type_with_parameter_key(path, base.span)?;
                let key = key
                    .or_else(|| match &constructor {
                        Type::Named { path, .. } => Some(self.path_to_string(path)),
                        Type::Generic { name, .. } => Some(name.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        invalid(
                            base.span,
                            "an applied type argument has no declared constructor identity",
                        )
                    })?;
                let (parameters, owner) = self.argument_constructor_parameters(&key, base.span)?;
                let arguments = self.elaborate_declared_arguments(
                    &parameters,
                    &owner,
                    args,
                    None,
                    annotation.span,
                )?;
                // The ordinary nominal/alias/variant constructor is shared;
                // checked values are passed into it, never synthesized again.
                self.apply_generic_type_arguments(annotation, Some(arguments))?
            }
            TypeKind::Tuple(elements) => Type::tuple(
                elements
                    .iter()
                    .map(|element| self.checked_protocol_type_argument(element))
                    .collect::<Result<_>>()?,
            ),
            TypeKind::Function {
                params,
                return_type,
                ..
            } => {
                let checked_params = params
                    .iter()
                    .map(|parameter| self.checked_protocol_type_argument(parameter))
                    .collect::<Result<List<_>>>()?;
                let checked_result = self.checked_protocol_type_argument(return_type)?;
                let mut resolved = self.ast_to_type(annotation)?;
                let Type::Function {
                    params,
                    return_type,
                    ..
                } = &mut resolved
                else {
                    return Err(invalid(
                        annotation.span,
                        "function argument lowering changed its type constructor",
                    ));
                };
                *params = checked_params;
                **return_type = checked_result;
                resolved
            }
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. }
            | TypeKind::Pointer { inner, .. }
            | TypeKind::VolatilePointer { inner, .. }
            | TypeKind::Ownership { inner, .. }
            | TypeKind::GenRef { inner }
            | TypeKind::Slice(inner)
            | TypeKind::Array { element: inner, .. }
            | TypeKind::Refined { base: inner, .. } => {
                let checked_inner = self.checked_protocol_type_argument(inner)?;
                let mut resolved = self.ast_to_type(annotation)?;
                match &mut resolved {
                    Type::Reference { inner, .. }
                    | Type::CheckedReference { inner, .. }
                    | Type::UnsafeReference { inner, .. }
                    | Type::Pointer { inner, .. }
                    | Type::VolatilePointer { inner, .. }
                    | Type::Ownership { inner, .. }
                    | Type::GenRef { inner, .. }
                    | Type::Slice { element: inner }
                    | Type::Array { element: inner, .. }
                    | Type::Refined { base: inner, .. } => {
                        **inner = checked_inner;
                    }
                    _ => {
                        return Err(invalid(
                            annotation.span,
                            "structural argument lowering changed its type constructor",
                        ));
                    }
                }
                resolved
            }
            TypeKind::Path(_)
            | TypeKind::Unit
            | TypeKind::Never
            | TypeKind::Bool
            | TypeKind::Int
            | TypeKind::Float
            | TypeKind::Char
            | TypeKind::Text => self.ast_to_type(annotation)?,
            _ => {
                return Err(invalid(
                    annotation.span,
                    "this type argument requires a scoped structural elaborator that preserves all parameter roles",
                ));
            }
        };
        let resolved = self.unifier.normalize_aliases(&resolved);
        self.validate_elaborated_type_argument(&resolved, annotation.span)?;
        Ok(resolved)
    }

    /// Alias expansion can expose more constructors. Validate those against
    /// their own source headers, rather than treating Meta as a type because
    /// it is nested or because a syntactic alias was already checked.
    fn validate_elaborated_type_argument(&mut self, ty: &Type, span: Span) -> Result<()> {
        let _depth =
            super::ThreadLocalDepthGuard::new(&super::AST_TO_TYPE_DEPTH, 64).ok_or_else(|| {
                invalid(
                    span,
                    "type argument validation exceeds its structural limit",
                )
            })?;
        match ty {
            Type::Meta { .. } | Type::Unknown => Err(invalid(
                span,
                "a value or unknown witness cannot occupy a type parameter",
            )),
            Type::Named { path, args } => {
                self.validate_constructor_argument_roles(&self.path_to_string(path), args, span)
            }
            Type::Generic { name, args } => {
                self.validate_constructor_argument_roles(name, args, span)
            }
            Type::Tuple(elements) => {
                for element in elements {
                    self.validate_elaborated_type_argument(element, span)?;
                }
                Ok(())
            }
            Type::Record(fields) | Type::Variant(fields) => {
                for field in fields.values() {
                    self.validate_elaborated_type_argument(field, span)?;
                }
                Ok(())
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                for parameter in params {
                    self.validate_elaborated_type_argument(parameter, span)?;
                }
                self.validate_elaborated_type_argument(return_type, span)
            }
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Pointer { inner, .. }
            | Type::VolatilePointer { inner, .. }
            | Type::Ownership { inner, .. }
            | Type::GenRef { inner, .. }
            | Type::Slice { element: inner }
            | Type::Array { element: inner, .. }
            | Type::Refined { base: inner, .. }
            | Type::Forall { body: inner, .. } => {
                self.validate_elaborated_type_argument(inner, span)
            }
            Type::Unit
            | Type::Never
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Char
            | Type::Text
            | Type::Var(_)
            | Type::Universe { .. }
            | Type::Prop => Ok(()),
            _ => Err(invalid(
                span,
                "this elaborated type argument has no complete parameter-role traversal",
            )),
        }
    }

    fn validate_constructor_argument_roles(
        &mut self,
        key: &Text,
        args: &[Type],
        span: Span,
    ) -> Result<()> {
        if args.is_empty() {
            return Ok(());
        }
        let (parameters, owner) = self.argument_constructor_parameters(key, span)?;
        if parameters.len() != args.len() {
            return Err(invalid(
                span,
                format!("type constructor `{key}` lost its declared argument arity"),
            ));
        }
        let mut bindings = IndexMap::new();
        for (parameter, argument) in parameters.iter().zip(args) {
            let name = match &parameter.kind {
                GenericParamKind::Type { name, .. } => {
                    self.validate_elaborated_type_argument(argument, span)?;
                    name
                }
                GenericParamKind::Meta {
                    name,
                    ty,
                    refinement,
                } => {
                    if refinement.is_some() {
                        return Err(invalid(
                            span,
                            "refined protocol values require a checked refinement witness",
                        ));
                    }
                    let carrier =
                        self.in_argument_declaration_scope(&owner, &bindings, None, |checker| {
                            checker.ast_to_type(ty)
                        })?;
                    self.validate_elaborated_value_argument(
                        argument,
                        &self.unifier.normalize_aliases(&carrier),
                        span,
                    )?;
                    name
                }
                GenericParamKind::Const { name, ty } => {
                    let carrier =
                        self.in_argument_declaration_scope(&owner, &bindings, None, |checker| {
                            checker.ast_to_type(ty)
                        })?;
                    self.validate_elaborated_value_argument(
                        argument,
                        &self.unifier.normalize_aliases(&carrier),
                        span,
                    )?;
                    name
                }
                _ => {
                    return Err(invalid(
                        parameter.span,
                        "constructor parameter kind has no exact application evidence",
                    ));
                }
            };
            bindings.insert(name.name.clone(), argument.clone());
        }
        Ok(())
    }

    fn validate_elaborated_value_argument(
        &self,
        argument: &Type,
        carrier: &Type,
        span: Span,
    ) -> Result<()> {
        self.check_protocol_value_carrier(carrier, span)?;
        match argument {
            Type::Meta {
                value: Some(ConstValue::Int(value)),
                ty,
                refinement: None,
                ..
            } if ty.as_ref() == carrier && i64::try_from(*value).is_ok() => Ok(()),
            _ => Err(invalid(
                span,
                "value argument lacks its exact declared scalar evidence",
            )),
        }
    }

    fn argument_constructor_parameters(
        &mut self,
        key: &Text,
        span: Span,
    ) -> Result<(List<GenericParam>, Text)> {
        if let Some(declaration) = self
            .source_type_registration
            .completed
            .get(key)
            .or_else(|| self.source_type_registration.pending.get(key))
        {
            let owner = key.as_str().rsplit_once('.').map_or("", |(owner, _)| owner);
            return Ok((declaration.generics.clone(), owner.into()));
        }
        let (owner, name) = key.as_str().rsplit_once('.').unwrap_or(("", key.as_str()));
        let previous = self.current_module_path.clone();
        self.set_current_module_path(owner);
        let source = self.declaring_source_type_declaration(name);
        self.set_current_module_path(previous);
        if let Some((declaration, declared_owner)) = source {
            let declared_key = if declared_owner.is_empty() {
                declaration.name.name.clone()
            } else {
                format!("{declared_owner}.{}", declaration.name.name).into()
            };
            if declared_key == *key {
                return Ok((declaration.generics, declared_owner));
            }
        }
        if let Some(declaration) = self
            .core_metadata
            .as_ref()
            .and_then(|metadata| Self::metadata_type_with_declaring_key(metadata, key))
            .and_then(|descriptor| descriptor.source_declaration.as_ref())
        {
            return Ok((declaration.generics.clone(), owner.into()));
        }
        Err(invalid(
            span,
            format!("type constructor `{key}` lacks its declaration-owned parameter header"),
        ))
    }

    fn check_protocol_value_carrier(&self, carrier: &Type, span: Span) -> Result<()> {
        // This is the same checked scalar domain as VBC ConstValue. Refinements
        // and other scalar sorts cannot be inferred from an erased ABI carrier.
        if matches!(carrier, Type::Refined { .. }) {
            return Err(invalid(
                span,
                "refined protocol values require a checked refinement witness",
            ));
        }
        if carrier != &Type::Int {
            return Err(invalid(
                span,
                "protocol value carrier has no exact checked scalar representation",
            ));
        }
        Ok(())
    }

    fn checked_protocol_value_argument(
        &mut self,
        argument: &GenericArg,
        carrier: &Type,
        span: Span,
    ) -> Result<Type> {
        self.check_protocol_value_carrier(carrier, span)?;
        let expression = match argument {
            GenericArg::Const(expression) => expression.clone(),
            GenericArg::Type(annotation) => {
                let verum_ast::TypeKind::Path(path) = &annotation.kind else {
                    return Err(invalid(
                        annotation.span,
                        "a value parameter requires a value argument",
                    ));
                };
                verum_ast::Expr::new(verum_ast::ExprKind::Path(path.clone()), annotation.span)
            }
            _ => return Err(invalid(span, "a value parameter requires a value argument")),
        };
        verum_ast::checked_const::validate_i64_expression(&expression).map_err(|span| {
            invalid(
                span,
                "value argument has an unsupported literal type or exceeds its structural limit",
            )
        })?;
        let value = self
            .const_eval
            .eval_array_count(&expression, self.current_module_path.as_str())
            .map_err(|error| {
                invalid(
                    expression.span,
                    format!(
                        "value argument is not an exactly checked compile-time integer: {error}"
                    ),
                )
            })?;
        Ok(Type::meta_value(
            ConstValue::Int(i128::from(value)),
            carrier.clone(),
        ))
    }
}
