//! Shared declaration ordering for compiler and editor analysis.

use super::TypeChecker;
use crate::TypeError;
use verum_ast::{ItemKind, Module};
use verum_common::{List, Set, Text};

impl TypeChecker {
    /// Use the compilation target for item, implementation and body selection.
    pub fn set_cfg_target(&mut self, target: verum_ast::cfg::TargetConfig) {
        self.property_inferrer.set_cfg_target(target.clone());
        self.cfg_evaluator = verum_ast::cfg::CfgEvaluator::with_config(target);
    }

    /// Prepare a source module before checking its bodies with [`Self::check_item`].
    ///
    /// The caller supplies module identity, registry, lazy resolver, builtins and
    /// language/backend policy. This is declaration preparation, not whole-project
    /// verification: callers must also report recovered and deferred diagnostics.
    #[must_use]
    pub fn prepare_module_declarations(&mut self, module: &Module) -> List<TypeError> {
        self.seed_module_properties(&module.items);
        self.prepare_checked_count_file(module);
        self.register_declared_meta_macros(module);
        let parent = self.current_module_path().clone();
        let mut errors = self.prepare_module_imports(module, parent.as_str());
        errors.extend(self.prepare_module_types(module));
        errors.extend(self.prepare_module_values(module));
        errors
    }

    /// Establish inline modules and mounts before resolving local declarations.
    ///
    /// The staged entry points let the compiler retain its stdlib registration and
    /// coherence/profile gates between declaration passes without duplicating them.
    #[must_use]
    pub fn prepare_module_imports(
        &mut self,
        module: &Module,
        inline_parent_path: &str,
    ) -> List<TypeError> {
        for item in &module.items {
            if let ItemKind::Module(decl) = &item.kind {
                self.pre_register_module_public(decl, inline_parent_path);
            }
        }
        let owner = self.current_module_path().clone();
        let mut errors = List::new();
        for item in &module.items {
            if let ItemKind::Mount(decl) = &item.kind {
                // Never hold a registry read guard while resolving an import:
                // lazy loading writes this same registry. Refresh each snapshot
                // so declarations loaded by an earlier mount are visible.
                let registry = self.module_registry().read().clone();
                if let Err(error) = self.process_import(decl, owner.as_str(), &registry) {
                    errors.push(error);
                }
            }
        }
        errors
    }

    /// Resolve forward types, contexts, protocols and implementations, in order.
    #[must_use]
    pub fn prepare_module_types(&mut self, module: &Module) -> List<TypeError> {
        let mut selected = module.clone();
        selected.items.retain(|item| self.cfg_evaluator.should_include(&item.attributes));
        self.source_type_registration.local_declarations.push((self.current_module_path.clone(), verum_common::Shared::new(selected)));
        let result = self.prepare_module_types_in_source_scope(module);
        self.source_type_registration.local_declarations.pop();
        result
    }

    fn prepare_module_types_in_source_scope(&mut self, module: &Module) -> List<TypeError> {
        let previous_placeholders: Set<Text> = self
            .verify_no_placeholders()
            .into_iter()
            .filter_map(|error| match error {
                TypeError::UnresolvedPlaceholder { name, .. } => Some(name),
                _ => None,
            })
            .collect();
        self.register_all_type_names(&module.items);
        // Protocol signatures may require contexts declared later in the file.
        // Establish only their declared identities here; full method contracts
        // are elaborated below after nominal type definitions are available.
        for item in &module.items {
            if let ItemKind::Context(decl) = &item.kind {
                self.context_resolver.register_protocol_as_context(decl.name.name.clone());
            }
        }
        self.set_alias_scope_from_items(&module.items);
        let mut errors = List::new();
        let mut resolution_stack = List::new();
        for item in &module.items {
            if let ItemKind::Type(decl) = &item.kind {
                if let Err(error) = self.resolve_type_definition(decl, &mut resolution_stack) {
                    errors.push(error);
                }
            }
        }
        errors.extend(self.verify_no_placeholders().into_iter().filter(|error| {
            !matches!(error, TypeError::UnresolvedPlaceholder { name, .. }
                if previous_placeholders.contains(name))
        }));
        for item in &module.items {
            if let ItemKind::Context(decl) = &item.kind {
                self.register_stdlib_context_full(decl.name.name.clone(), decl.clone());
            }
        }
        for item in &module.items {
            if let ItemKind::Protocol(decl) = &item.kind {
                if let Err(error) = self.register_protocol(decl) {
                    errors.push(error);
                }
            }
        }
        for item in &module.items {
            if let ItemKind::Impl(decl) = &item.kind {
                if let Err(error) = self.register_impl_block(decl) {
                    errors.push(error);
                }
            }
        }
        errors
    }

    /// Register function, extern and constant signatures before checking bodies.
    #[must_use]
    pub fn prepare_module_values(&mut self, module: &Module) -> List<TypeError> {
        let mut selected = module.clone();
        selected.items.retain(|item| self.cfg_evaluator.should_include(&item.attributes));
        self.source_type_registration.local_declarations.push((self.current_module_path.clone(), verum_common::Shared::new(selected)));
        let result = self.prepare_module_values_in_source_scope(module);
        self.source_type_registration.local_declarations.pop();
        result
    }

    fn prepare_module_values_in_source_scope(&mut self, module: &Module) -> List<TypeError> {
        let mut errors = List::new();
        for item in &module.items {
            if let ItemKind::Function(decl) = &item.kind {
                if let Err(error) = self.register_function_signature(decl) {
                    errors.push(error);
                }
            }
        }
        for item in &module.items {
            if let ItemKind::ExternBlock(block) = &item.kind {
                for decl in &block.functions {
                    if let Err(error) = self.register_function_signature(decl) {
                        errors.push(error);
                    }
                }
            }
        }
        for item in &module.items {
            if let ItemKind::Const(decl) = &item.kind {
                self.pre_register_const(decl);
            }
        }
        errors
    }
}
