//! Lexical parameter identities shared by ordinary and method body checking.
use super::TypeChecker;
use crate::Result;
use verum_ast::{FunctionDecl, ty::GenericParamKind};
use verum_common::List;

impl TypeChecker {
    /// Named bodies have independent move/borrow and callable state. Moving
    /// this state preserves the enclosing body without cloning its trackers.
    /// Declaration registries and the unifier remain shared across the check.
    pub(super) fn with_function_body_context<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let env_depth = self.ctx.env.scope_depth();
        let context_scope = self.context_checker.enter_function_scope();
        let stage_scope = self.stage_checker.take_body_context();
        let affine = self.affine_tracker.new_scope();
        let saved = (
            std::mem::replace(&mut self.affine_tracker, affine),
            std::mem::replace(
                &mut self.borrow_tracker,
                crate::aliasing::BorrowTracker::new(),
            ),
            std::mem::take(&mut self.init_tracker),
            self.current_function_return_type.take(),
            self.current_function_name.take(),
            self.current_function_return_span.take(),
            std::mem::take(&mut self.current_function_params),
            self.current_function_throws.take(),
            self.generator_context.take(),
            std::mem::replace(&mut self.in_async_context, false),
            std::mem::replace(&mut self.in_unsafe_context, false),
            std::mem::replace(&mut self.in_call_arg_context, false),
            self.current_function_contexts.take(),
            std::mem::take(&mut self.current_source_contexts),
            std::mem::take(&mut self.current_function_call_sites),
            self.current_function_being_checked.take(),
            std::mem::take(&mut self.current_function_stage),
            std::mem::replace(&mut self.current_function_is_transparent, false),
        );
        let pending = (
            self.pending_ambiguity.len(),
            self.pending_protocol_static_calls.len(),
            self.pending_method_overloads.len(),
        );
        let result = operation(self);
        self.ctx.env.restore_scope_depth(env_depth);
        self.context_checker.exit_function_scope(context_scope);
        self.stage_checker.restore_body_context(stage_scope);
        (
            self.affine_tracker,
            self.borrow_tracker,
            self.init_tracker,
            self.current_function_return_type,
            self.current_function_name,
            self.current_function_return_span,
            self.current_function_params,
            self.current_function_throws,
            self.generator_context,
            self.in_async_context,
            self.in_unsafe_context,
            self.in_call_arg_context,
            self.current_function_contexts,
            self.current_source_contexts,
            self.current_function_call_sites,
            self.current_function_being_checked,
            self.current_function_stage,
            self.current_function_is_transparent,
        ) = saved;
        if result.is_err() {
            self.pending_ambiguity.truncate(pending.0);
            self.pending_protocol_static_calls.truncate(pending.1);
            self.pending_method_overloads.truncate(pending.2);
        }
        result
    }

    pub(super) fn with_function_parameter_scope<T>(
        &mut self,
        function: &FunctionDecl,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let mut saved = List::new();
        for parameter in &function.generics {
            let name = match &parameter.kind {
                GenericParamKind::Type { name, .. }
                | GenericParamKind::Meta { name, .. }
                | GenericParamKind::Const { name, .. }
                | GenericParamKind::HigherKinded { name, .. }
                | GenericParamKind::KindAnnotated { name, .. } => &name.name,
                _ => continue,
            };
            saved.push((name.clone(), self.ctx.type_defs.get(name).cloned()));
        }
        let inherited = self.decl_param_frames.last().cloned().unwrap_or_default();
        self.push_decl_param_frame(inherited);
        let result = operation(self);
        self.pop_decl_param_frame();
        for (name, previous) in saved {
            match previous {
                Some(ty) => {
                    self.ctx.type_defs.insert(name, ty);
                }
                None => {
                    self.ctx.type_defs.remove(&name);
                }
            }
        }
        result
    }
}
