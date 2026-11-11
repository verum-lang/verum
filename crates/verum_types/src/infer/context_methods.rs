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
}

impl TypeChecker {
    pub(super) fn register_context_method_source(&mut self, name: Text) {
        self.context_method_signatures.insert(
            name,
            ContextMethodSignatures {
                source: self.current_module_path.clone(),
                methods: Map::new(),
            },
        );
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
            &return_type, &Map::new(), &[], &crate::ty::Substitution::new(),
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
