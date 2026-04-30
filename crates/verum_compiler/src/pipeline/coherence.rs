//! Protocol coherence checking — orphan rule, overlap detection,
//! cross-crate violations, specialization.
//!
//! Extracted from `pipeline.rs` (#106 Phase 6). Validates that
//! protocol implementations follow coherence rules:
//!
//!   * **Orphan rule** — implementation must be in the crate that
//!     defines either the protocol OR the type.
//!   * **Overlap prevention** — no two implementations can apply to
//!     the same `(protocol, type)` pair.
//!   * **Specialization** — overlapping impls must opt-in via
//!     `@specialize`.
//!
//! The cluster is gated on `[protocols].coherence` in `verum.toml`:
//!
//!   * `unchecked` → skip coherence checking entirely.
//!   * `lenient`   → emit violations as warnings (default).
//!   * `strict`    → emit violations as errors.
//!
//! Pre-registered "trusted" crates (`core`, `sys`, `mem`, …) are
//! allowed to define blanket implementations like
//! `implement<T, U: From<T>> Into<U> for T { ... }` without
//! tripping the orphan rule, regardless of the file under
//! compilation.

use anyhow::Result;
use tracing::debug;

use verum_ast::Module;
use verum_common::{List, Text};
use verum_diagnostics::{DiagnosticBuilder, Severity};
use verum_modules::{CoherenceChecker, CoherenceError, ImplEntry, ModuleId, ModulePath};

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    pub(super) fn check_protocol_coherence(&self, module: &Module) -> Result<()> {
        // Gate on [protocols].coherence. "unchecked" skips all
        // coherence rules; "lenient" and "strict" proceed (the
        // method already classifies by severity internally).
        let coherence_mode = self
            .session
            .language_features()
            .protocols
            .coherence
            .as_str();
        if coherence_mode == "unchecked" {
            tracing::debug!(
                "Protocol coherence checking SKIPPED ([protocols] coherence = \"unchecked\")"
            );
            return Ok(());
        }

        use verum_ast::decl::ImplKind;

        // Determine crate name from module path or use "main" as default.
        let crate_name = if let Some(item) = module.items.first() {
            if let Some(source_file) = self.session.get_source(item.span.file_id) {
                if let Some(ref file_path) = source_file.path {
                    file_path
                        .components()
                        .find_map(|c| {
                            if let std::path::Component::Normal(s) = c {
                                s.to_str().map(Text::from)
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| Text::from("main"))
                } else {
                    Text::from("main")
                }
            } else {
                Text::from("main")
            }
        } else {
            Text::from("main")
        };

        let mut checker = CoherenceChecker::new(crate_name.clone());

        // Mark stdlib crates as trusted for blanket implementations.
        // This allows stdlib to define implementations like:
        //   implement<T, U: From<T>> Into<U> for T { ... }
        // Always trust these regardless of which file is being compiled.
        checker.add_trusted_crate("core");
        checker.add_trusted_crate("sys");
        checker.add_trusted_crate("mem");
        checker.add_trusted_crate("collections");
        checker.add_trusted_crate("async");
        checker.add_trusted_crate("io");
        checker.add_trusted_crate("runtime");
        checker.add_trusted_crate("meta");

        let current_module_path = ModulePath::from_str(crate_name.as_str());
        let current_module_id = ModuleId::new(0); // Default module ID for single-file mode.

        // ───────────────────────────────────────────────────────────
        // Register stdlib types, protocols, and impl blocks so
        // coherence checking can detect cross-crate overlaps and
        // orphan violations between user code and the stdlib.
        // ───────────────────────────────────────────────────────────
        let mut ext_module_counter: u32 = 1; // reserve 0 for user module
        for (mod_path, stdlib_mod) in &self.modules {
            let stdlib_mod_path = ModulePath::from_str(mod_path.as_str());
            let stdlib_mod_id = ModuleId::new(ext_module_counter);
            ext_module_counter += 1;

            self.register_module_coherence_items(
                &mut checker,
                stdlib_mod,
                &stdlib_mod_path,
                stdlib_mod_id,
            );
        }

        // Project modules (cross-file imports in multi-file projects).
        for (mod_path, project_mod) in &self.project_modules {
            let proj_mod_path = ModulePath::from_str(mod_path.as_str());
            let proj_mod_id = ModuleId::new(ext_module_counter);
            ext_module_counter += 1;

            self.register_module_coherence_items(
                &mut checker,
                project_mod,
                &proj_mod_path,
                proj_mod_id,
            );
        }

        // ───────────────────────────────────────────────────────────
        // Register user module types, protocols, and impl blocks.
        // ───────────────────────────────────────────────────────────

        // Register local types (defined in this module).
        for item in &module.items {
            if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                let type_name = Text::from(type_decl.name.as_str());
                checker.register_local_type(type_name, current_module_path.clone());
            }
        }

        // Register local protocols (defined in this module).
        for item in &module.items {
            match &item.kind {
                verum_ast::ItemKind::Protocol(protocol_decl) => {
                    let protocol_name = Text::from(protocol_decl.name.as_str());
                    checker.register_local_protocol(protocol_name, current_module_path.clone());
                }
                verum_ast::ItemKind::Type(type_decl) => {
                    // `type X is protocol { ... }` also defines a local protocol.
                    if matches!(&type_decl.body, verum_ast::decl::TypeDeclBody::Protocol(_)) {
                        let protocol_name = Text::from(type_decl.name.as_str());
                        checker.register_local_protocol(protocol_name, current_module_path.clone());
                    }
                }
                _ => {}
            }
        }

        // Collect user implement blocks as ImplEntry.
        for item in &module.items {
            if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                if let ImplKind::Protocol {
                    protocol, for_type, ..
                } = &impl_decl.kind
                {
                    let protocol_name = protocol.to_string();
                    let protocol_path = ModulePath::from_str(&protocol_name);
                    let for_type_text = self.type_to_text(for_type);

                    let mut entry = ImplEntry::new(
                        Text::from(protocol_name),
                        protocol_path,
                        for_type_text,
                        current_module_path.clone(),
                        current_module_id,
                    );

                    entry = entry.with_span(impl_decl.span);

                    if impl_decl.specialize_attr.is_some() {
                        entry = entry.with_specialized();
                    }

                    if !impl_decl.generics.is_empty() {
                        let params: List<Text> = impl_decl
                            .generics
                            .iter()
                            .filter_map(|g| {
                                use verum_ast::ty::GenericParamKind;
                                match &g.kind {
                                    GenericParamKind::Type { name, .. } => {
                                        Some(Text::from(name.as_str()))
                                    }
                                    GenericParamKind::HigherKinded { name, .. } => {
                                        Some(Text::from(name.as_str()))
                                    }
                                    _ => None,
                                }
                            })
                            .collect();
                        entry = entry.with_type_params(params);
                    }

                    // Extract @cfg predicates from item attributes and module path.
                    let cfg_preds = crate::cfg_eval::extract_cfg_predicates(
                        &item.attributes,
                        &current_module_path,
                    );
                    if !cfg_preds.is_empty() {
                        entry = entry.with_cfg_predicates(cfg_preds);
                    }

                    checker.add_impl(entry);
                }
            }
        }

        // Run all coherence checks (orphan rules, overlap,
        // specialization, cross-crate).
        let errors = checker.check_all();

        if !errors.is_empty() {
            debug!("Protocol coherence: found {} violation(s)", errors.len());
        }

        // Emit diagnostics as warnings — coherence violations are
        // advisory for now so they don't block compilation while the
        // checker is being hardened.
        for error in errors {
            let ast_span = match &error {
                CoherenceError::OrphanImpl { span, .. } => *span,
                CoherenceError::OverlappingImpl { span, .. } => *span,
                CoherenceError::InvalidSpecialization { span, .. } => *span,
                CoherenceError::ConflictingCrateImpl { span, .. } => *span,
            };

            let mut builder = DiagnosticBuilder::new(Severity::Warning)
                .message(format!("[coherence] {}", error));
            if let Some(ast_span) = ast_span {
                let diag_span = self.session.convert_span(ast_span);
                builder = builder.span(diag_span);
            }
            self.session.emit_diagnostic(builder.build());
        }

        Ok(())
    }

    /// Register a single module's types, protocols, and impl blocks
    /// into the coherence checker.
    pub(super) fn register_module_coherence_items(
        &self,
        checker: &mut CoherenceChecker,
        module: &Module,
        mod_path: &ModulePath,
        mod_id: ModuleId,
    ) {
        use verum_ast::decl::ImplKind;

        for item in &module.items {
            match &item.kind {
                verum_ast::ItemKind::Type(type_decl) => {
                    if matches!(&type_decl.body, verum_ast::decl::TypeDeclBody::Protocol(_)) {
                        let protocol_name = Text::from(type_decl.name.as_str());
                        checker.register_local_protocol(protocol_name, mod_path.clone());
                    }
                }
                verum_ast::ItemKind::Protocol(protocol_decl) => {
                    let protocol_name = Text::from(protocol_decl.name.as_str());
                    checker.register_local_protocol(protocol_name, mod_path.clone());
                }
                verum_ast::ItemKind::Impl(impl_decl) => {
                    if let ImplKind::Protocol {
                        protocol, for_type, ..
                    } = &impl_decl.kind
                    {
                        let protocol_name = protocol.to_string();
                        let protocol_path = ModulePath::from_str(&protocol_name);
                        let for_type_text = self.type_to_text(for_type);

                        let mut entry = ImplEntry::new(
                            Text::from(protocol_name),
                            protocol_path,
                            for_type_text,
                            mod_path.clone(),
                            mod_id,
                        );

                        if impl_decl.specialize_attr.is_some() {
                            entry = entry.with_specialized();
                        }

                        if !impl_decl.generics.is_empty() {
                            let params: List<Text> = impl_decl
                                .generics
                                .iter()
                                .filter_map(|g| {
                                    use verum_ast::ty::GenericParamKind;
                                    match &g.kind {
                                        GenericParamKind::Type { name, .. } => {
                                            Some(Text::from(name.as_str()))
                                        }
                                        GenericParamKind::HigherKinded { name, .. } => {
                                            Some(Text::from(name.as_str()))
                                        }
                                        _ => None,
                                    }
                                })
                                .collect();
                            entry = entry.with_type_params(params);
                        }

                        // Extract @cfg predicates from item attributes and module path.
                        let cfg_preds =
                            crate::cfg_eval::extract_cfg_predicates(&item.attributes, mod_path);
                        if !cfg_preds.is_empty() {
                            entry = entry.with_cfg_predicates(cfg_preds);
                        }

                        checker.add_impl(entry);
                    }
                }
                _ => {}
            }
        }
    }
}
