//! Default methods are checked as universally quantified declaration bodies.
use super::TypeChecker;
use crate::{
    Result, Type, TypeError,
    context::TypeScheme,
    protocol::{Protocol, ProtocolBound, ProtocolConstraints},
    ty::{Substitution, TypeVar},
};
use verum_ast::{
    FunctionDecl, GenericParam, Ident, Path, PathSegment, Span,
    decl::{ProtocolItem, ProtocolItemKind},
    ty::GenericParamKind,
};
use verum_common::{List, Map, Text};

fn invalid(span: Span, message: impl Into<Text>) -> TypeError {
    TypeError::OtherWithCodeSpanned {
        code: "E405".into(),
        msg: message.into(),
        span,
    }
}

impl TypeChecker {
    pub(super) fn check_protocol_default_bodies(
        &mut self,
        name: &Ident,
        parameters: &List<GenericParam>,
        items: &[ProtocolItem],
    ) -> Result<()> {
        if !items.iter().any(|item| matches!(&item.kind, ProtocolItemKind::Function { decl, default_impl } if decl.body.is_some() || default_impl.is_some())) {
            return Ok(());
        }
        let identity = self.declared_type_key(&name.name);
        let protocol = self
            .protocol_checker
            .read()
            .get_protocol_definition(&identity)
            .cloned()
            .ok_or_else(|| invalid(name.span, "default body has no exact declaring protocol"))?;
        for item in items {
            let ProtocolItemKind::Function { decl, default_impl } = &item.kind else {
                continue;
            };
            if decl.body.is_none() && default_impl.is_none() {
                continue;
            }
            if !self.cfg_evaluator.should_include(&decl.attributes) {
                continue;
            }
            let mut function = decl.clone();
            if function.body.is_none() {
                function.body = default_impl.clone();
            }
            self.with_protocol_body_environment(parameters, &function, |checker| {
                checker.install_protocol_body_owner(&protocol, parameters, &function)?;
                checker.check_function(&function)
            })?;
        }
        Ok(())
    }

    fn install_protocol_body_owner(
        &mut self,
        protocol: &Protocol,
        parameters: &[GenericParam],
        function: &FunctionDecl,
    ) -> Result<()> {
        let ProtocolConstraints::Declared {
            self_type,
            parameters: variables,
            parameter_bounds,
            ..
        } = &protocol.constraints
        else {
            return Err(invalid(
                function.span,
                "default body requires a represented protocol owner telescope",
            ));
        };
        if parameters.len() != variables.len() {
            return Err(invalid(
                function.span,
                "default body owner telescope differs from its declaration",
            ));
        }
        let receiver = TypeVar::fresh();
        let mut substitution = Substitution::new();
        substitution.insert(*self_type, Type::Var(receiver));
        let mut frame = indexmap::IndexMap::new();
        let mut arguments = List::new();
        let mut rigid = List::new();
        for (parameter, original) in parameters.iter().zip(variables) {
            let GenericParamKind::Type { name, .. } = &parameter.kind else {
                return Err(invalid(
                    parameter.span,
                    "default body owner binder has no represented type domain",
                ));
            };
            let variable = TypeVar::fresh();
            let ty = Type::Var(variable);
            substitution.insert(*original, ty.clone());
            frame.insert(name.name.clone(), ty.clone());
            arguments.push(ty.clone());
            self.ctx.define_type(name.name.clone(), ty.clone());
            self.ctx
                .env
                .insert_type_parameter_mirror(name.name.clone(), TypeScheme::mono(ty));
            rigid.push((variable, name.name.clone(), false, None));
        }
        self.push_decl_param_frame(frame);
        self.set_current_self_type(Some(Type::Var(receiver)));
        // The canonical owner comes from the selected declaration, never its
        // last path component or a sibling's same-spelled protocol.
        let path = Path::new(
            protocol
                .name
                .as_str()
                .split('.')
                .map(|part| PathSegment::Name(Ident::new(part, function.span)))
                .collect(),
            function.span,
        );
        self.register_type_var_bounds(
            receiver,
            List::from_iter([ProtocolBound {
                protocol: path,
                args: arguments,
                associated_types: Map::new(),
                is_negative: false,
            }]),
        );
        for clause in parameter_bounds {
            let clause = clause
                .try_apply_subst(&substitution)
                .map_err(|message| invalid(function.span, message))?;
            if let Type::Var(variable) = &clause.ty {
                let mut bounds = self.get_type_var_bounds(variable);
                bounds.extend(clause.bounds.iter().cloned());
                self.register_type_var_bounds(*variable, bounds);
            }
            self.source_type_registration
                .projection_assumptions
                .push(clause);
        }
        for (variable, _, bounded, _) in &mut rigid {
            *bounded = !self.get_type_var_bounds(variable).is_empty();
        }
        rigid.push((receiver, "Self".into(), true, None));
        self.unifier.enter_rigid_scope(rigid);
        Ok(())
    }

    /// Isolate method-local state on every Result path, without undoing source
    /// dependency registrations performed by the ordinary body checker.
    fn with_protocol_body_environment<T>(
        &mut self,
        owner: &[GenericParam],
        function: &FunctionDecl,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let mut names = List::new();
        for parameter in owner.iter().chain(&function.generics) {
            let name = match &parameter.kind {
                GenericParamKind::Type { name, .. }
                | GenericParamKind::Meta { name, .. }
                | GenericParamKind::Const { name, .. }
                | GenericParamKind::HigherKinded { name, .. }
                | GenericParamKind::KindAnnotated { name, .. } => &name.name,
                _ => continue,
            };
            if !names.iter().any(|(previous, _)| previous == name) {
                names.push((name.clone(), self.ctx.type_defs.get(name).cloned()));
            }
        }
        let env_depth = self.ctx.env.scope_depth();
        self.ctx.enter_scope();
        let rigid = self.unifier.rigid_snapshot();
        let projection_assumptions =
            std::mem::take(&mut self.source_type_registration.projection_assumptions);
        let frames = std::mem::take(&mut self.decl_param_frames);
        let context = self
            .context_checker
            .body_scope_snapshot(&function.name.name);
        let stage = self.stage_checker.body_scope_snapshot(&function.name.name);
        let properties = self
            .property_inferrer
            .context()
            .body_scope_snapshot(&function.name.name);
        let contract = self.function_contracts.get(&function.name.name).cloned();
        let ambiguity = self.pending_ambiguity.len();
        let statics = self.pending_protocol_static_calls.len();
        let overloads = self.pending_method_overloads.len();
        let saved = (
            self.current_self_type.clone(),
            self.in_impl_block,
            self.in_inherent_impl,
            self.type_var_bounds.clone(),
            self.hkt_type_var_by_name.clone(),
            self.type_var_type_bounds.clone(),
            self.ctx.type_var_bounds.clone(),
            self.current_function_return_type.clone(),
            self.current_function_name.clone(),
            self.current_function_return_span,
            self.current_function_params.clone(),
            self.current_function_throws.clone(),
            self.generator_context.clone(),
            self.in_async_context,
            self.current_function_contexts.clone(),
            self.current_function_call_sites.clone(),
            self.current_function_being_checked.clone(),
            self.current_function_stage,
            self.current_function_is_transparent,
            self.affine_tracker.clone(),
            self.borrow_tracker.clone(),
            self.init_tracker.clone(),
            self.refinement_evidence.clone(),
            self.mutable_bindings.clone(),
        );
        self.in_impl_block = true;
        self.in_inherent_impl = false;
        let result = operation(self);
        (
            self.current_self_type,
            self.in_impl_block,
            self.in_inherent_impl,
            self.type_var_bounds,
            self.hkt_type_var_by_name,
            self.type_var_type_bounds,
            self.ctx.type_var_bounds,
            self.current_function_return_type,
            self.current_function_name,
            self.current_function_return_span,
            self.current_function_params,
            self.current_function_throws,
            self.generator_context,
            self.in_async_context,
            self.current_function_contexts,
            self.current_function_call_sites,
            self.current_function_being_checked,
            self.current_function_stage,
            self.current_function_is_transparent,
            self.affine_tracker,
            self.borrow_tracker,
            self.init_tracker,
            self.refinement_evidence,
            self.mutable_bindings,
        ) = saved;
        self.set_current_self_type(self.current_self_type.clone());
        self.ctx.env.restore_scope_depth(env_depth);
        for (name, ty) in names {
            match ty {
                Some(ty) => {
                    self.ctx.type_defs.insert(name, ty);
                }
                None => {
                    self.ctx.type_defs.remove(&name);
                }
            }
        }
        self.unifier.exit_rigid_scope(rigid);
        self.decl_param_frames = frames;
        self.source_type_registration.projection_assumptions = projection_assumptions;
        self.context_checker
            .restore_body_scope(&function.name.name, context);
        self.stage_checker
            .restore_body_scope(&function.name.name, stage);
        self.property_inferrer
            .context_mut()
            .restore_body_scope(&function.name.name, properties);
        match contract {
            Some(contract) => {
                self.function_contracts
                    .insert(function.name.name.clone(), contract);
            }
            None => {
                self.function_contracts.remove(&function.name.name);
            }
        }
        self.pending_ambiguity.truncate(ambiguity);
        self.pending_protocol_static_calls.truncate(statics);
        self.pending_method_overloads.truncate(overloads);
        result
    }
}
