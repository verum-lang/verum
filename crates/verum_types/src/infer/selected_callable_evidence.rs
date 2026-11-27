//! Refine an open callback contract using the source declarations selected by
//! ordinary expression inference. Check every reachable operation before using
//! the result. An unresolved or dynamic call keeps the callback unrestricted.
use super::*;

impl TypeChecker {
    pub(in crate::infer) fn checked_source_calls_are_synchronous(
        &mut self,
        expression: &Expr,
    ) -> bool {
        if self.source_type_registration.checking_callable_evidence {
            return false;
        }
        let local = self.property_inferrer.infer_local_operations(expression);
        let mut pending = List::new();
        let mut calls = Calls {
            checker: self,
            bound: PropertyRequirement::AtMost(local),
            source: None,
            obligations: Some(&mut pending),
        };
        calls.visit_expr(expression);
        if calls.bound.may_be_async() || pending.is_empty() {
            return false;
        }

        let selected_sites: List<_> = pending.iter().map(|call| (call.span, call.target.clone())).collect();
        let graph = Shared::new(parking_lot::RwLock::new(
            self.module_registry.read().clone(),
        ));
        let mut prepared: Map<Text, TypeChecker> = Map::new();
        let mut visited = Map::new();
        let mut checked_implementations = Set::new();
        let mut remaining = MAX_SOURCE_CALL_STEPS;
        while let Some(CallObligation { target: (owner, declaration), source: selected, .. }) = pending.pop() {
            let Some(next) = remaining.checked_sub(1) else {
                return false;
            };
            remaining = next;
            let key = (owner.clone(), selected.identity());
            if visited.contains_key(&key) {
                continue;
            }
            if has_implicit_call_computation(&declaration)
                || declaration.body.is_none()
            {
                return false;
            }
            if !prepared.contains_key(&owner) {
                let source = selected.source.module();
                let Some(next) = remaining.checked_sub(source.items.len()) else {
                    return false;
                };
                remaining = next;
                let Some(context) = self.prepare_callable_evidence(&owner, source, &graph) else {
                    return false;
                };
                prepared.insert(owner.clone(), context);
            }
            let checker = prepared.get_mut(&owner).expect("prepared callable owner");
            let item = &selected.source.module().items[selected.item];
            let checked = match (&item.kind, selected.member) {
                (ItemKind::Impl(implementation), Some(member)) if matches!(implementation.kind, verum_ast::decl::ImplKind::Inherent(_)) => {
                    let only_member = implementation.items[member].clone();
                    let mut item = item.clone();
                    if let ItemKind::Impl(block) = &mut item.kind {
                        block.items = List::from_single(only_member);
                    }
                    checker.check_item(&item)
                }
                (ItemKind::Impl(implementation), _) => {
                    // Protocol conformance and its published identity belong to the
                    // complete implementation, including its associated types.
                    if checked_implementations.insert((owner.clone(), selected.item)) {
                        let Some(next) = remaining.checked_sub(implementation.items.len()) else {
                            return false;
                        };
                        remaining = next;
                        checker.check_item(item)
                    } else {
                        Ok(())
                    }
                }
                _ => checker.check_item(item),
            };
            if checked.is_err() || !checker.take_deferred_errors().is_empty()
                || !checker.diagnostic_sources().is_empty()
                || checker.diagnostics().iter().any(|diagnostic| diagnostic.is_error()) {
                return false;
            }
            if !checker.collect_callable_evidence(&declaration, &mut pending) {
                return false;
            }
            visited.insert(key, selected);
        }
        for (span, target) in selected_sites {
            self.type_registry
                .publish_synchronous_call(span, &target);
        }
        true
    }

    fn collect_callable_evidence(
        &mut self,
        function: &FunctionDecl,
        pending: &mut List<CallObligation>,
    ) -> bool {
        let Some(body) = &function.body else {
            return false;
        };
        let local = self.property_inferrer.infer_function_body(body);
        let mut calls = Calls {
            checker: self,
            bound: PropertyRequirement::AtMost(local),
            source: None,
            obligations: Some(pending),
        };
        match body {
            verum_ast::FunctionBody::Block(block) => calls.visit_block(block),
            verum_ast::FunctionBody::Expr(expression) => calls.visit_expr(expression),
        }
        !calls.bound.may_be_async()
    }

    fn prepare_callable_evidence(
        &self,
        owner: &Text,
        source: &verum_ast::Module,
        graph: &Shared<parking_lot::RwLock<verum_modules::ModuleRegistry>>,
    ) -> Option<TypeChecker> {
        let mut checker = match &self.core_metadata {
            Some(metadata) => TypeChecker::new_with_core(metadata.clone()),
            None => TypeChecker::with_minimal_context(),
        };
        checker.source_type_registration.checking_callable_evidence = true;
        checker.set_cfg_target(self.cfg_evaluator.config().clone());
        checker.analysis_control = self.analysis_control.clone();
        checker.set_cubical_enabled(self.unifier.cubical_enabled());
        checker.set_dependent_enabled(self.dependent_enabled);
        checker.set_higher_kinded_enabled(self.higher_kinded_enabled);
        checker.set_universe_poly_enabled(self.universe_poly_enabled);
        checker.set_coinductive_enabled(self.coinductive_enabled);
        checker.set_quotient_enabled(self.quotient_enabled);
        checker.set_instance_search_enabled(self.instance_search_enabled);
        checker.set_coherence_check_depth(self.coherence_check_depth);
        checker.set_higher_kinded_protocols_enabled(self.higher_kinded_protocols_enabled());
        checker.set_generic_associated_types_enabled(self.generic_associated_types_enabled());
        {
            let policy = self.protocol_checker.read();
            checker.set_protocol_resolution_strategy(policy.resolution_strategy().clone());
            checker.set_protocol_blanket_impls(policy.blanket_impls_allowed());
            checker.set_protocol_coherence_mode(policy.coherence_mode());
        }
        checker.register_builtins();
        checker.set_current_module_path(owner.clone());
        checker.set_module_registry(graph.clone());
        checker.register_stdlib_types_for_module(source);
        let mut module = source.clone();
        if verum_ast::prelude::implicit_prelude_path(&module)
            .is_some_and(|path| graph.read().get_by_path(path).is_some())
        {
            verum_ast::prelude::inject_implicit_prelude_mount(&mut module);
        }
        if !checker.prepare_module_declarations(&module).is_empty()
            || !checker.take_deferred_errors().is_empty()
            || !checker.diagnostic_sources().is_empty()
            || checker
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.is_error())
        {
            return None;
        }
        checker.enter_module_declarations(&module);
        Some(checker)
    }

    /// Resolve in the checker's call-site environment before traversing a
    /// provider's body. Generated declarations can share names and spans.
    pub(super) fn selected_callable_source(
        &self,
        span: Span,
        owner: &Text,
        selected: &FunctionDecl,
    ) -> Option<SourceFunction> {
        use verum_ast::decl::{ImplItemKind, ImplKind};
        let source = self
            .source_type_registration
            .local_declarations
            .iter()
            .rev()
            .find(|(path, _)| path == owner)
            .map(|(_, ast)| CallableSource::Lexical(ast.clone()))
            .or_else(|| {
                self.module_registry
                    .read()
                    .get_by_path_aliased(owner)
                    .map(CallableSource::Registered)
            })?;
        let mut found = None;
        for (item, entry) in source.module().items.iter().enumerate() {
            if !self.cfg_evaluator.should_include(&entry.attributes) {
                continue;
            }
            match &entry.kind {
                ItemKind::Function(function)
                    if function.span == selected.span && function == selected =>
                {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(SourceFunction {
                        source: source.clone(),
                        item,
                        member: None,
                    });
                }
                ItemKind::Impl(implementation) => {
                    for (index, entry) in implementation.items.iter().enumerate() {
                        if let ImplItemKind::Function(function) = &entry.kind
                            && function.span == selected.span
                            && function == selected
                            && self.cfg_evaluator.should_include(&entry.attributes)
                        {
                            if !matches!(implementation.kind, ImplKind::Inherent(_))
                                && !self.selected_protocol_implementation(span, owner, implementation, &function.name.name)
                            {
                                return None;
                            }
                            if found.is_some() {
                                return None;
                            }
                            found = Some(SourceFunction {
                                source: source.clone(),
                                item,
                                member: Some(index),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        found
    }

    fn selected_protocol_implementation(
        &self,
        span: Span,
        owner: &Text,
        source: &verum_ast::decl::ImplDecl,
        method: &Text,
    ) -> bool {
        let Some(Type::Reference { inner, .. }) = self.type_registry.declared_receiver(span) else {
            return false;
        };
        let receiver = self.unifier.normalize_aliases(&self.unifier.apply(inner));
        let protocols = self.protocol_checker.read();
        let Ok(Some(selected)) = protocols.source_impl(owner, source) else { return false };
        // An open receiver can acquire further applicable implementations after
        // substitution. For a closed receiver, use ordinary dispatch below: it
        // instantiates the implementation and proves its where clauses.
        if receiver.contains_type(|part| matches!(part,
            Type::Var(_) | Type::RecordField { .. } | Type::Array { size: None, .. }
            | Type::Meta { value: crate::ty::ValueTerm::Parameter(_) | crate::ty::ValueTerm::Inference(_), .. }
        )) || self.contains_unresolved_projection(&receiver) {
            return false;
        }
        let Ok((premises, callables)) = self.method_dispatch_premises(span) else { return false };
        let environment = self.source_type_registration.proof_environment(&premises, &callables);
        let candidates = protocols.get_implementations_in_environment(&receiver, environment);
        let mut declaring = candidates.into_iter().filter(|implementation| implementation.methods.contains_key(method));
        declaring.next().is_some_and(|candidate| std::ptr::eq(candidate, selected)) && declaring.next().is_none()
    }
}
