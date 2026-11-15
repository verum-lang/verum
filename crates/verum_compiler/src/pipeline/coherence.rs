//! Protocol coherence checking — orphan rule, overlap detection,
//! cross-crate violations, specialization.
//!
//! Extracted from `pipeline.rs` (#106 Phase 6). Validates that
//! protocol implementations follow coherence rules:
//!
//!  * **Orphan rule** — implementation must be in the crate that
//!  defines either the protocol OR the type.
//!  * **Overlap prevention** — no two implementations can apply to
//!  the same `(protocol, type)` pair.
//!  * **Specialization** — overlapping impls must opt-in via
//!  `@specialize`.
//!
//! The cluster is gated on `[protocols].coherence` in `verum.toml`:
//!
//!  * `unchecked` → skip coherence checking entirely.
//!  * `lenient` and `strict` currently emit violations as warnings.
//!    `strict` is the configured default, but enforcing its error severity
//!    remains separate from source-owner and overlap identification.
//!
//! Pre-registered "trusted" crates (`core`, `sys`, `mem`, …) are
//! allowed to define blanket implementations like
//! `implement<T, U: From<T>> Into<U> for T { ... }` without
//! tripping the orphan rule, regardless of the file under
//! compilation.

use anyhow::Result;
use tracing::debug;

use verum_ast::Module;
use verum_common::{List, Map, Maybe, Text};
use verum_diagnostics::{DiagnosticBuilder, Severity};
use verum_modules::{CoherenceChecker, CoherenceError, ImplEntry, ModuleId, ModulePath};

use super::CompilationPipeline;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum SourceUnitIdentity {
    File(std::path::PathBuf, u32, u32),
    Session(verum_ast::FileId, u32, u32),
}

impl<'s> CompilationPipeline<'s> {
    /// File aliases may be loaded with different FileIds. Anonymous/unregistered
    /// ASTs have no session identity and must not be collapsed by a dummy FileId.
    pub(super) fn source_unit_identity(&self, module: &Module) -> Maybe<SourceUnitIdentity> {
        let source = self.session.get_source(module.file_id)?;
        Some(
            match source
                .path
                .as_ref()
                .and_then(|path| path.canonicalize().ok())
            {
                Some(path) => SourceUnitIdentity::File(path, module.span.start, module.span.end),
                None => {
                    SourceUnitIdentity::Session(module.file_id, module.span.start, module.span.end)
                }
            },
        )
    }

    /// Compare the registered snapshots, never reread mutable files. Distinct
    /// editor revisions can share a physical path and the same byte extent.
    pub(super) fn same_source_snapshot(
        &self,
        left: verum_ast::FileId,
        right: verum_ast::FileId,
    ) -> bool {
        match (
            self.session.get_source(left),
            self.session.get_source(right),
        ) {
            (Some(left), Some(right)) => left.source == right.source,
            _ => false,
        }
    }

    pub(super) fn check_protocol_coherence(
        &self,
        module: &Module,
        current_owner: &str,
    ) -> Result<()> {
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

        // The checker and registry have already selected the declaration owner.
        // A filesystem prefix (e.g. /Volumes or /Users) is not a cog name.
        let current_module_path = ModulePath::from_str(current_owner);
        let crate_name = current_module_path
            .segments()
            .first()
            .cloned()
            .unwrap_or_default();

        let mut checker = CoherenceChecker::new(crate_name.clone());

        // Mark stdlib crates as trusted for blanket implementations.
        // This allows stdlib to define implementations like:
        //  implement<T, U: From<T>> Into<U> for T { ... }
        // Always trust these regardless of which file is being compiled.
        checker.add_trusted_crate("core");
        checker.add_trusted_crate("sys");
        checker.add_trusted_crate("mem");
        checker.add_trusted_crate("collections");
        checker.add_trusted_crate("async");
        checker.add_trusted_crate("io");
        checker.add_trusted_crate("runtime");
        checker.add_trusted_crate("meta");

        let current_module_id = ModuleId::new(0); // Default module ID for single-file mode.

        // ───────────────────────────────────────────────────────────
        // Register stdlib types, protocols, and impl blocks so
        // coherence checking can detect cross-crate overlaps and
        // orphan violations between user code and the stdlib.
        // ───────────────────────────────────────────────────────────
        let mut registered_sources = Map::new();
        if let Some(identity) = self.source_unit_identity(module) {
            registered_sources.insert(identity, (module.file_id, Text::from(current_owner)));
        }
        let mut ext_module_counter: u32 = 1; // reserve 0 for the checked source
        for (selected_owner, dependency) in self.modules.iter().chain(self.project_modules.iter()) {
            let owner =
                crate::owned_source_semantics::declared_owner(dependency, selected_owner.as_str());
            if let Some(identity) = self.source_unit_identity(dependency) {
                if let Some((file, previous_owner)) = registered_sources.get(&identity) {
                    anyhow::ensure!(
                        previous_owner == &owner
                            && self.same_source_snapshot(*file, dependency.file_id),
                        "conflicting source snapshots or owners for {owner}"
                    );
                    continue;
                }
                registered_sources.insert(identity, (dependency.file_id, owner.clone()));
            }
            let dependency_path = ModulePath::from_str(owner.as_str());
            let dependency_id = ModuleId::new(ext_module_counter);
            ext_module_counter += 1;
            self.register_module_coherence_items(
                &mut checker,
                dependency,
                &dependency_path,
                dependency_id,
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

        self.register_module_coherence_items(
            &mut checker,
            module,
            &current_module_path,
            current_module_id,
        );

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

            let mut builder =
                DiagnosticBuilder::new(Severity::Warning).message(format!("[coherence] {}", error));
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
                        )
                        .with_span(impl_decl.span);

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

#[cfg(test)]
#[path = "../../tests/unit/coherence_source_identity.rs"]
mod source_identity_tests;
