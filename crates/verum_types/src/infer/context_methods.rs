//! Context calls use the same declaration schemes and obligations as functions.
use super::{InferResult, TypeChecker};
use crate::context::TypeScheme;
use crate::{Result, Type, TypeError};
use verum_ast::ty::GenericArg;
use verum_ast::{Expr, ExprKind, Ident, Span};
use verum_common::{List, Map, Text};

pub(super) struct ContextMethodSignatures {
    source: Text,
    methods: Map<Text, TypeScheme>,
    /// Context protocols dispatch through the checked protocol declaration.
    /// The availability view below is not another signature authority.
    protocol: Option<Text>,
}

impl TypeChecker {
    pub(super) fn register_context_method_source(&mut self, name: Text) {
        self.context_method_signatures.insert(
            name,
            ContextMethodSignatures {
                source: self.current_module_path.clone(),
                methods: Map::new(),
                protocol: None,
            },
        );
    }

    pub(super) fn publish_context_protocol_binding(
        &mut self,
        declaration: &verum_ast::TypeDecl,
        identity: &Text,
        binding: Text,
    ) {
        let verum_ast::TypeDeclBody::Protocol(body) = &declaration.body else {
            return;
        };
        if !body.is_context {
            return;
        }
        // This projection supplies lexical availability and method names only.
        // Associated members, supers, kinds and all method constraints remain
        // in the checked protocol selected by its canonical identity.
        let availability = verum_ast::ContextDecl {
            visibility: declaration.visibility.clone(),
            is_async: false,
            name: declaration.name.clone(),
            generics: declaration.generics.clone(),
            methods: body
                .items
                .iter()
                .filter_map(|item| match &item.kind {
                    verum_ast::decl::ProtocolItemKind::Function { decl, .. } => Some(decl.clone()),
                    _ => None,
                })
                .collect(),
            associated_types: List::new(),
            associated_consts: List::new(),
            sub_contexts: List::new(),
            span: declaration.span,
        };
        self.context_checker
            .register_context(binding.clone(), availability.clone());
        self.context_declarations
            .insert(binding.clone(), availability);
        self.context_resolver
            .register_protocol_as_context(binding.clone());
        self.context_resolver.register_context_type(
            binding.clone(),
            Type::Named {
                path: Self::text_to_path(identity),
                args: List::new(),
            },
        );
        self.context_method_signatures.insert(
            binding,
            ContextMethodSignatures {
                source: identity
                    .rsplit_once('.')
                    .map_or("", |(owner, _)| owner)
                    .into(),
                methods: Map::new(),
                protocol: Some(identity.clone()),
            },
        );
    }

    pub(super) fn context_protocol_binding(&self, name: &str) -> Option<&Text> {
        let identity = self
            .context_method_signatures
            .get(&Text::from(name))?
            .protocol
            .as_ref()?;
        // A later concrete type of the same spelling must still shadow the
        // context. An imported protocol's qualified head is its own identity.
        match self.ctx.lookup_type(name) {
            Some(Type::Named { path, .. }) => {
                let selected = self.path_to_string(path);
                (selected == *identity || selected.as_str() == name).then_some(identity)
            }
            None => None,
            _ => None,
        }
    }

    pub(super) fn try_context_method_call(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        type_args: &List<GenericArg>,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<InferResult>> {
        let Some(ty) = self.try_context_method_value(receiver, method, type_args, span)? else {
            return Ok(None);
        };
        let Type::Function {
            params,
            return_type,
            contexts,
            ..
        } = ty
        else {
            return Err(TypeError::Other(
                "context method has no callable signature".into(),
            ));
        };
        if params.len() != args.len() {
            return Err(self.arity_error(method.name.clone(), params.len(), args.len(), span));
        }
        if let Some(contexts) = contexts {
            self.check_callable_contexts(&contexts, span)?;
        }
        for (argument, parameter) in args.iter().zip(&params) {
            let expected = self.unifier.apply(parameter);
            self.check_expr(argument, &expected)?;
        }
        let result = self.resolve_call_output(
            &return_type,
            &Map::new(),
            &[],
            &crate::ty::Substitution::new(),
            span,
        )?;
        Ok(Some(InferResult::new(result)))
    }

    pub(super) fn try_context_method_value(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        type_args: &List<GenericArg>,
        span: Span,
    ) -> Result<Option<Type>> {
        let ExprKind::Path(path) = &receiver.kind else {
            return Ok(None);
        };
        let Some(ident) = path.as_ident() else {
            return Ok(None);
        };
        let requirement = self
            .current_source_contexts
            .iter()
            .find(|requirement| {
                !requirement.is_negative
                    && (requirement
                        .alias
                        .as_ref()
                        .or(requirement.name.as_ref())
                        .is_some_and(|alias| alias.name == ident.name)
                        || requirement
                            .path
                            .as_ident()
                            .is_some_and(|name| name.name == ident.name))
            })
            .cloned();
        let context_name = requirement
            .as_ref()
            .and_then(|r| r.path.as_ident())
            .map_or_else(|| ident.name.clone(), |name| name.name.clone());
        if self.user_type_shadows_context(&context_name) {
            return Ok(None);
        }
        let Some(declaration) = self.context_declarations.get(&context_name).cloned() else {
            return Ok(None);
        };
        if !self.context_checker.is_available(&context_name)
            && !self.context_resolver.is_lenient_contexts()
        {
            return Err(TypeError::MissingContext {
                context: context_name,
                span,
            });
        }
        if let Some(identity) = self.context_protocol_binding(&context_name).cloned() {
            let owner_arguments = requirement
                .as_ref()
                .map(|r| {
                    r.args
                        .iter()
                        .cloned()
                        .map(GenericArg::Type)
                        .collect::<List<_>>()
                })
                .unwrap_or_default();
            let path = Self::text_to_path(&identity);
            let arguments = self.protocol_application_arguments(&path, &owner_arguments, None)?;
            let receiver = Type::Named {
                path,
                args: arguments.clone(),
            };
            let selected = self
                .applied_protocol_method_in_hierarchy(
                    &identity,
                    &arguments,
                    &receiver,
                    &method.name,
                    span,
                )?
                .ok_or_else(|| TypeError::UndefinedContextMethod {
                    context: context_name.clone(),
                    method: method.name.clone(),
                    span,
                })?;
            return self
                .instantiate_applied_protocol_method(&selected, &receiver, type_args, span)
                .map(Some);
        }
        self.context_checker
            .check_context_call(&context_name, &method.name, span)?;
        let Some(mut function) = declaration
            .methods
            .iter()
            .find(|m| m.name.name == method.name)
            .cloned()
        else {
            return Err(TypeError::OtherWithCodeSpanned {
                code: "E102".into(),
                msg: format!("context '{context_name}' has no method '{}'", method.name).into(),
                span,
            });
        };
        function.params.retain(|parameter| !parameter.is_self());

        // Resolve the owner's arguments in the caller's scope, before entering
        // the declaration's binders. Method arguments occupy separate slots.
        let owner_arguments: List<GenericArg> = requirement
            .as_ref()
            .map(|r| r.args.iter().cloned().map(GenericArg::Type).collect())
            .unwrap_or_default();
        let owner_count = declaration
            .generics
            .iter()
            .filter(|parameter| !parameter.is_implicit)
            .count();
        if owner_arguments.len() != owner_count {
            return Err(TypeError::OtherWithCodeSpanned {
                code: "E408".into(),
                msg: format!(
                    "context '{context_name}' requires {} type arguments, got {}",
                    owner_count,
                    owner_arguments.len()
                )
                .into(),
                span,
            });
        }

        let cached = self
            .context_method_signatures
            .get(&context_name)
            .and_then(|signatures| signatures.methods.get(&method.name))
            .cloned();
        let scheme = if let Some(scheme) = cached {
            scheme
        } else {
            let source = self
                .context_method_signatures
                .get(&context_name)
                .map(|s| s.source.clone())
                .unwrap_or_else(|| self.current_module_path.clone());
            let mut generics = declaration.generics.clone();
            generics.extend(function.generics.iter().cloned());
            function.generics = generics;
            // The provider's receiver is implicit; an unannotated requirement
            // returns Unit, unlike a function body whose result can be inferred.
            if function.return_type.is_none() {
                function.return_type = Some(verum_ast::Type::unit(function.span));
            }
            let owner = std::mem::replace(&mut self.current_module_path, source);
            let resolving = std::mem::replace(&mut self.resolving_source_signature, true);
            let self_type = self.current_self_type.take();
            self.push_decl_param_frame(indexmap::IndexMap::new());
            let qualified_method = format!("{}.{}", declaration.name.name, method.name).into();
            let result = self.source_function_scheme_named(&function, &qualified_method);
            self.pop_decl_param_frame();
            self.current_self_type = self_type;
            self.resolving_source_signature = resolving;
            self.current_module_path = owner;
            let scheme = result?;
            if let Some(signatures) = self.context_method_signatures.get_mut(&context_name) {
                signatures
                    .methods
                    .insert(method.name.clone(), scheme.clone());
            }
            scheme
        };
        let instance = self.instantiate_value_instance(&scheme, span)?;
        if let Some(slots) = &instance.explicit_method_vars {
            let owner_slots = slots.iter().take(owner_count).cloned().collect();
            self.bind_explicit_method_arguments(
                Some(&owner_slots),
                &owner_arguments,
                method,
                span,
            )?;
            let method_slots = slots.iter().skip(owner_count).cloned().collect();
            self.bind_explicit_method_arguments(Some(&method_slots), type_args, method, span)?;
        }
        Ok(Some(self.unifier.apply(&instance.ty)))
    }
}
