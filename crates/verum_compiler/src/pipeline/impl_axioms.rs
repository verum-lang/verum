//! Model-theoretic discharge of protocol axioms at impl sites.
//!
//! Extracted from `pipeline.rs` (#106 Phase 3). For every `implement
//! P for T { ... }` block in the module, this surface collects P's
//! axioms (Self-substituted to T's concrete ops) and discharges each
//! via:
//!
//!   * An explicit `proof X by tactic;` clause inside the impl block.
//!   * `ProofSearchEngine::auto_prove` fallback.
//!
//! Unverified obligations surface as diagnostics at warning severity
//! by default; the session option `model_verification_level` can
//! elevate them to errors.
//!
//! Reference specification: `docs/architecture/model-theoretic-semantics.md`.

use anyhow::Result;
use tracing::info;

use verum_ast::Module;
use verum_diagnostics::{DiagnosticBuilder, Severity};

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    pub(super) fn verify_impl_axioms_for_module(&self, module: &Module) -> Result<()> {
        use crate::phases::proof_verification::verify_impl_axioms;
        use verum_ast::decl::{ImplKind, TypeDeclBody};

        let mut impl_count = 0u32;
        let mut verified_axioms = 0u32;
        let mut unverified_axioms = 0u32;

        for item in module.items.iter() {
            let verum_ast::ItemKind::Impl(impl_decl) = &item.kind else {
                continue;
            };
            let ImplKind::Protocol { protocol, .. } = &impl_decl.kind else {
                // Inherent impls have no axioms to discharge.
                continue;
            };

            // Resolve the protocol AST by path. Protocols declared in
            // the same module are searchable directly; cross-module
            // protocols are looked up via the module registry.
            let protocol_name = match protocol.segments.last().and_then(|seg| match seg {
                verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str()),
                _ => None,
            }) {
                Some(n) => n,
                None => continue,
            };

            let protocol_decl = match self.find_protocol_decl(module, protocol_name) {
                Some(pd) => pd,
                None => continue,
            };

            // Only proceed if the protocol body is actually a protocol
            // (not a stray alias with matching name).
            if !matches!(protocol_decl.body, TypeDeclBody::Protocol(_)) {
                continue;
            }

            impl_count += 1;
            let report = verify_impl_axioms(impl_decl, &protocol_decl);
            verified_axioms += report.verified.len() as u32;
            unverified_axioms += report.unverified.len() as u32;

            for failure in report.unverified.iter() {
                let diag_msg = format!(
                    "model verification: `implement {} for <type>` does not discharge axiom `{}` ({})",
                    report.protocol_name, failure.axiom_name, failure.reason,
                );
                let diag = DiagnosticBuilder::new(Severity::Warning)
                    .message(diag_msg)
                    .build();
                self.session.emit_diagnostic(diag);
            }
        }

        if impl_count > 0 {
            info!(
                "Model verification: {} impl blocks, {} axioms verified, {} unverified",
                impl_count, verified_axioms, unverified_axioms
            );
        }

        Ok(())
    }

    /// Look up a protocol's TypeDecl by name. Searches the given
    /// module first, then falls back to the module registry for
    /// cross-module lookup.
    pub(super) fn find_protocol_decl(
        &self,
        module: &Module,
        protocol_name: &str,
    ) -> Option<verum_ast::decl::TypeDecl> {
        // 1. Search this module.
        for item in module.items.iter() {
            if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                if type_decl.name.name.as_str() == protocol_name {
                    return Some(type_decl.clone());
                }
            }
        }
        // 2. Cross-module lookup: walk every loaded module's items.
        for (_path, loaded) in self.modules.iter() {
            for item in loaded.items.iter() {
                if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                    if type_decl.name.name.as_str() == protocol_name {
                        return Some(type_decl.clone());
                    }
                }
            }
        }
        None
    }
}
