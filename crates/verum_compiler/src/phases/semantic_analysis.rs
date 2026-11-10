//! Phase 4: Semantic Analysis
//!
//! Bidirectional type checking with refinement types.
//!
//! This phase supports two modes:
//! 1. **StdlibBootstrap**: Compiling stdlib itself (uses minimal context with builtins only)
//! 2. **NormalBuild**: Compiling user code (loads stdlib types from embedded stdlib.vbca)
//!
//! Phase 4: Semantic analysis. Name resolution with profile awareness,
//! bidirectional type checking, refinement subsumption (syntactic + SMT),
//! reference validation (&mut exclusive, & shared), context resolution.
//! Supports both normal compilation (pre-loaded stdlib) and bootstrap mode.

use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;
use verum_common::{List, Map, Text};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};
use verum_types::TypeChecker;
use verum_types::core_metadata::CoreMetadata;

use super::{
    CompilationPhase, PhaseData, PhaseInput, PhaseMetrics, PhaseOutput, VerifiedContractRegistry,
};

pub struct SemanticAnalysisPhase {
    /// Verified contracts from Phase 3a (if available)
    verified_contracts: Option<VerifiedContractRegistry>,
    source_owners: Map<verum_ast::FileId, (Text, Text)>,
    target: verum_ast::cfg::TargetConfig,
    module_registry: Option<crate::owned_source_semantics::SourceRegistry>,
    /// Stdlib metadata for type checking (None = StdlibBootstrap mode)
    stdlib_metadata: Option<Arc<CoreMetadata>>,
    /// Number of user modules (from the end of the modules list).
    /// Retained for API compatibility; every supplied source body is checked.
    /// None means all modules are treated as user code.
    user_module_count: Option<usize>,
    /// Whether cubical normalization is enabled on the unifier
    /// (sourced from `[types] cubical` in `verum.toml`). Default on.
    cubical_enabled: bool,
    dependent_enabled: bool,
    higher_kinded_enabled: bool,
    universe_poly_enabled: bool,
    coinductive_enabled: bool,
    quotient_enabled: bool,
    instance_search_enabled: bool,
    coherence_check_depth: u32,
    /// `[protocols].resolution_strategy` — most_specific (default) /
    /// first_declared / error. Routed to ProtocolChecker.
    protocol_resolution_strategy: verum_common::Text,
    /// `[protocols].blanket_impls` — when false, ProtocolChecker
    /// excludes type-variable for_types (blanket impls) from
    /// candidate sets.
    protocol_blanket_impls: bool,
    /// `[protocols].coherence` — strict (default) | lenient |
    /// unchecked. Routed to ProtocolChecker.coherence_mode at
    /// the start of `execute()`.
    protocol_coherence_mode: verum_common::Text,
    /// `[protocols].higher_kinded_protocols` — when false (the
    /// manifest default), a protocol declaring an HKT generic
    /// parameter (e.g. `protocol Functor<F<_>>`) is rejected at
    /// registration time. Routed to TypeChecker.higher_kinded_
    /// protocols_enabled at the start of `execute()`.
    protocol_higher_kinded_protocols: bool,
    /// `[protocols].generic_associated_types` — when false (the
    /// manifest default), a protocol declaring a generic
    /// associated type (`type Item<T>`) is rejected at
    /// registration time. Routed to TypeChecker.generic_
    /// associated_types_enabled at the start of `execute()`.
    protocol_generic_associated_types: bool,
}

impl SemanticAnalysisPhase {
    pub(crate) fn with_source_owners(mut self, owners: Map<verum_ast::FileId, (Text, Text)>) -> Self {
        self.source_owners = owners;
        self
    }

    pub fn with_module_registry(mut self, registry: verum_common::Shared<parking_lot::RwLock<verum_modules::ModuleRegistry>>) -> Self {
        self.module_registry = Some(registry);
        self
    }

    pub(crate) fn with_target(mut self, target: verum_ast::cfg::TargetConfig) -> Self {
        self.target = target;
        self
    }

    pub(crate) fn with_language_features(mut self, features: &crate::language_features::LanguageFeatures) -> Self {
        self.cubical_enabled = features.types.cubical;
        self.dependent_enabled = features.types.dependent;
        self.higher_kinded_enabled = features.types.higher_kinded;
        self.universe_poly_enabled = features.types.universe_polymorphism;
        self.coinductive_enabled = features.types.coinductive;
        self.quotient_enabled = features.types.quotient;
        self.instance_search_enabled = features.types.instance_search;
        self.coherence_check_depth = features.types.coherence_check_depth;
        self.protocol_resolution_strategy = features.protocols.resolution_strategy.clone();
        self.protocol_blanket_impls = features.protocols.blanket_impls;
        self.protocol_coherence_mode = features.protocols.coherence.clone();
        self.protocol_higher_kinded_protocols = features.protocols.higher_kinded_protocols;
        self.protocol_generic_associated_types = features.protocols.generic_associated_types;
        self
    }

    pub(crate) fn select_owned_sources(
        &self,
        modules: &[verum_ast::Module],
    ) -> anyhow::Result<List<(Text, Text, verum_ast::Module)>> {
        let owned: List<_> = modules.iter().map(|module| {
            let (owner, path) = self.source_owners.get(&module.file_id).cloned()
                .unwrap_or_else(|| (crate::owned_source_semantics::declared_owner(module, "cog"), format!("source file {:?}", module.file_id).into()));
            (owner, path, module)
        }).collect();
        let sources: List<_> = owned.iter().map(|(owner, path, module)| (owner.as_str(), path.as_str(), *module)).collect();
        crate::owned_source_semantics::select_source_roster(&sources, &self.target)
    }

    /// Admit original staged source before any value materialization. The
    /// temporary graph preserves the caller's aliases and external modules;
    /// it is shared across all checkers and never published as lowered metadata.
    pub(crate) fn admit_owned_source_roster(
        &self,
        sources: &[(&str, &str, &verum_ast::Module)],
        session: &crate::Session,
    ) -> anyhow::Result<()> {
        let selected = crate::owned_source_semantics::select_source_roster(sources, &self.target)?;
        let graph = verum_common::Shared::new(parking_lot::RwLock::new(session.module_registry().read().clone()));
        crate::owned_source_semantics::register_sources(&graph,
            selected.iter().map(|(owner, path, module)| (owner.as_str(), path.as_str(), module)))?;
        for (owner, path, module) in &selected {
            let mut checker = self.configured_checker();
            if let Err(errors) = crate::owned_source_semantics::check_source_types(
                &mut checker, owner, module, &graph,
                |error| error.to_diagnostic_with_span(Some(|span| session.convert_span(span))),
            ) {
                let details = errors.iter().map(|diagnostic| {
                    let mut line = String::new();
                    if let Some(span) = diagnostic.primary_span() {
                        line.push_str(&format!("{span}: "));
                    }
                    if let Some(code) = diagnostic.code() {
                        line.push_str(code);
                        line.push(' ');
                    }
                    line.push_str(diagnostic.message());
                    line
                }).collect::<Vec<_>>().join("\n");
                anyhow::bail!("semantic checking failed in {owner} ({path}): {details}");
            }
        }
        Ok(())
    }

    pub(crate) fn configured_checker(&self) -> TypeChecker {
        // Mode selection:
        // - StdlibBootstrap (stdlib_metadata = None): Uses minimal context + builtins only
        // - NormalBuild (stdlib_metadata = Some): Loads stdlib types from pre-compiled metadata
        let mut phase_checker = match &self.stdlib_metadata {
            Some(stdlib) => {
                tracing::info!(
                    "Semantic analysis with stdlib metadata: {} types, {} protocols, {} functions",
                    stdlib.types.len(),
                    stdlib.protocols.len(),
                    stdlib.functions.len()
                );
                // O(1) Arc clone — TypeChecker shares the metadata
                // by reference rather than deep-copying ~3MB.
                TypeChecker::new_with_core(std::sync::Arc::clone(stdlib))
            }
            None => {
                // Compiling stdlib itself - use minimal context, types are registered
                // dynamically as stdlib .vr files are parsed.
                tracing::info!("Semantic analysis for stdlib compilation (minimal context)");
                TypeChecker::with_minimal_context()
            }
        };

        // Register built-in types and functions (Int, Bool, print, assert, etc.)
        // NOTE: In NormalBuild mode, these may already be loaded from stdlib metadata,
        // but register_builtins() is idempotent and ensures core intrinsics are available.
        phase_checker.register_builtins();

        // Post-cycle-break (2026-04-24): install SMT backend here — the
        // verum_types `RefinementChecker` no longer constructs one itself.
        phase_checker.set_smt_backend(Box::new(
            verum_smt::refinement_backend::RefinementZ3Backend::new(),
        ));

        // Apply ALL session-level feature gates to the type checker.
        phase_checker.set_cubical_enabled(self.cubical_enabled);
        phase_checker.set_dependent_enabled(self.dependent_enabled);
        phase_checker.set_higher_kinded_enabled(self.higher_kinded_enabled);
        phase_checker.set_universe_poly_enabled(self.universe_poly_enabled);
        phase_checker.set_coinductive_enabled(self.coinductive_enabled);
        phase_checker.set_quotient_enabled(self.quotient_enabled);
        phase_checker.set_instance_search_enabled(self.instance_search_enabled);
        phase_checker.set_coherence_check_depth(self.coherence_check_depth);

        // Apply `[protocols].*` manifest gates to the embedded
        // ProtocolChecker. Pre-fix the resolver hardcoded
        // "most_specific" + blanket=true regardless of manifest;
        // these calls thread the user's `[protocols].resolution_
        // strategy` and `[protocols].blanket_impls` settings into
        // the production `find_impl` path so they actually drive
        // resolution behaviour.
        phase_checker.set_protocol_resolution_strategy(self.protocol_resolution_strategy.clone());
        phase_checker.set_protocol_blanket_impls(self.protocol_blanket_impls);
        phase_checker.set_protocol_coherence_mode(
            verum_types::protocol::CoherenceMode::from_manifest_str(
                self.protocol_coherence_mode.as_str(),
            ),
        );
        phase_checker.set_higher_kinded_protocols_enabled(self.protocol_higher_kinded_protocols);
        phase_checker.set_generic_associated_types_enabled(self.protocol_generic_associated_types);

        phase_checker.set_cfg_target(self.target.clone());
        phase_checker
    }

    /// Create a new semantic analysis phase in StdlibBootstrap mode.
    ///
    /// Uses minimal context with builtins only (no external stdlib).
    pub fn new() -> Self {
        Self {
            verified_contracts: None,
            source_owners: Map::new(),
            module_registry: None,
            target: verum_ast::cfg::TargetConfig::host(),
            stdlib_metadata: None,
            user_module_count: None,
            cubical_enabled: true,
            dependent_enabled: true,
            higher_kinded_enabled: true,
            universe_poly_enabled: false,
            coinductive_enabled: true,
            quotient_enabled: true,
            instance_search_enabled: true,
            coherence_check_depth: 16,
            protocol_resolution_strategy: verum_common::Text::from("most_specific"),
            protocol_blanket_impls: true,
            protocol_coherence_mode: verum_common::Text::from("strict"),
            protocol_higher_kinded_protocols: false,
            protocol_generic_associated_types: false,
        }
    }

    /// Create a new semantic analysis phase in NormalBuild mode.
    ///
    /// Uses pre-compiled stdlib types from the provided metadata.
    pub fn with_core(stdlib: Arc<CoreMetadata>) -> Self {
        Self {
            verified_contracts: None,
            source_owners: Map::new(),
            module_registry: None,
            target: verum_ast::cfg::TargetConfig::host(),
            stdlib_metadata: Some(stdlib),
            user_module_count: None,
            cubical_enabled: true,
            dependent_enabled: true,
            higher_kinded_enabled: true,
            universe_poly_enabled: false,
            coinductive_enabled: true,
            quotient_enabled: true,
            instance_search_enabled: true,
            coherence_check_depth: 16,
            protocol_resolution_strategy: verum_common::Text::from("most_specific"),
            protocol_blanket_impls: true,
            protocol_coherence_mode: verum_common::Text::from("strict"),
            protocol_higher_kinded_protocols: false,
            protocol_generic_associated_types: false,
        }
    }

    /// Apply `[protocols].resolution_strategy` from manifest.
    /// Threaded through to `ProtocolChecker.resolution_strategy` at
    /// `phase_checker.set_protocol_resolution_strategy(...)`. Closes
    /// the inert-defense pattern around the field.
    pub fn with_protocol_resolution_strategy(
        mut self,
        strategy: impl Into<verum_common::Text>,
    ) -> Self {
        self.protocol_resolution_strategy = strategy.into();
        self
    }

    /// Apply `[protocols].blanket_impls` from manifest.
    pub fn with_protocol_blanket_impls(mut self, allowed: bool) -> Self {
        self.protocol_blanket_impls = allowed;
        self
    }

    /// Apply `[protocols].coherence` from manifest. Threaded through
    /// to `ProtocolChecker.coherence_mode` at
    /// `phase_checker.set_protocol_coherence_mode(...)`. Closes the
    /// inert-defense pattern around the field.
    pub fn with_protocol_coherence_mode(mut self, mode: impl Into<verum_common::Text>) -> Self {
        self.protocol_coherence_mode = mode.into();
        self
    }

    /// Apply `[protocols].higher_kinded_protocols` from manifest.
    /// Threaded through to `TypeChecker.higher_kinded_protocols_
    /// enabled` at `phase_checker.set_higher_kinded_protocols_
    /// enabled(...)`. Closes the inert-defense pattern at
    /// session.rs:590.
    pub fn with_protocol_higher_kinded_protocols(mut self, enabled: bool) -> Self {
        self.protocol_higher_kinded_protocols = enabled;
        self
    }

    /// Apply `[protocols].generic_associated_types` from manifest.
    /// Threaded through to `TypeChecker.generic_associated_types_
    /// enabled` at `phase_checker.set_generic_associated_types_
    /// enabled(...)`. Closes the inert-defense pattern at
    /// session.rs (#265).
    pub fn with_protocol_generic_associated_types(mut self, enabled: bool) -> Self {
        self.protocol_generic_associated_types = enabled;
        self
    }

    /// Enable or disable cubical-type normalization in the underlying
    /// type checker. Called by the pipeline from `Session::language_features`.
    pub fn with_cubical_enabled(mut self, enabled: bool) -> Self {
        self.cubical_enabled = enabled;
        self
    }

    pub fn with_dependent_enabled(mut self, enabled: bool) -> Self {
        self.dependent_enabled = enabled;
        self
    }

    /// Enable or disable higher-kinded types (`F: Type -> Type` in
    /// generic params). Closes the inert-defense pattern around
    /// `[types].higher_kinded` at the run_common_pipeline boundary —
    /// pre-fix the field landed on the manifest but flowed only via
    /// the full CompilationPipeline path; this makes it reachable
    /// from the API entry-point too.
    pub fn with_higher_kinded_enabled(mut self, enabled: bool) -> Self {
        self.higher_kinded_enabled = enabled;
        self
    }

    /// Enable or disable universe polymorphism (`Type(u)`,
    /// `@universe_poly`). Closes the inert-defense pattern around
    /// `[types].universe_polymorphism`.
    pub fn with_universe_poly_enabled(mut self, enabled: bool) -> Self {
        self.universe_poly_enabled = enabled;
        self
    }

    /// Enable or disable coinductive types (`codata` declarations
    /// with copatterns). Closes the inert-defense pattern around
    /// `[types].coinductive`.
    pub fn with_coinductive_enabled(mut self, enabled: bool) -> Self {
        self.coinductive_enabled = enabled;
        self
    }

    /// Enable or disable quotient types (HIT-based types for
    /// modular equivalence). Closes the inert-defense pattern
    /// around `[types].quotient`.
    pub fn with_quotient_enabled(mut self, enabled: bool) -> Self {
        self.quotient_enabled = enabled;
        self
    }

    /// Enable or disable automatic protocol-implementation
    /// resolution. Closes the inert-defense pattern around
    /// `[types].instance_search`.
    pub fn with_instance_search_enabled(mut self, enabled: bool) -> Self {
        self.instance_search_enabled = enabled;
        self
    }

    /// Set the maximum coherence-check depth for instance
    /// resolution. Closes the inert-defense pattern around
    /// `[types].coherence_check_depth` at the API entry-point.
    pub fn with_coherence_check_depth(mut self, depth: u32) -> Self {
        self.coherence_check_depth = depth;
        self
    }

    /// Create a semantic analysis phase with verified contracts
    pub fn with_contracts(mut self, registry: VerifiedContractRegistry) -> Self {
        self.verified_contracts = Some(registry);
        self
    }

    /// Set stdlib metadata (builder pattern)
    pub fn with_core_metadata(mut self, stdlib: Arc<CoreMetadata>) -> Self {
        self.stdlib_metadata = Some(stdlib);
        self
    }

    /// Set the number of user modules (from the end of the modules list).
    /// All supplied source modules remain subject to semantic checking.
    pub fn with_user_module_count(mut self, count: usize) -> Self {
        self.user_module_count = Some(count);
        self
    }
}

impl Default for SemanticAnalysisPhase {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilationPhase for SemanticAnalysisPhase {
    fn name(&self) -> &str {
        "Phase 4: Semantic Analysis"
    }

    fn description(&self) -> &str {
        "Bidirectional type checking (3x faster than Hindley-Milner)"
    }

    fn execute(&self, input: PhaseInput) -> Result<PhaseOutput, List<Diagnostic>> {
        let start = Instant::now();

        // Extract modules AND contracts (if available)
        let (modules, contracts_opt) = match &input.data {
            PhaseData::AstModulesWithContracts {
                modules,
                verification_results,
            } => (modules, Some(verification_results.clone())),
            PhaseData::AstModules(modules) => (modules, None),
            _ => {
                let diag = DiagnosticBuilder::new(Severity::Error)
                    .message("Invalid input for semantic analysis phase".to_string())
                    .build();
                return Err(List::from(vec![diag]));
            }
        };

        // Log if contracts are available
        if let Some(ref contracts) = contracts_opt {
            tracing::info!(
                "Semantic analysis with {} verified contracts",
                contracts.verified_contracts.len()
            );
            tracing::debug!(
                "Contract breakdown: {} functions, {} types, {} protocols",
                contracts.stats.functions_with_contracts,
                contracts.stats.type_invariants_verified,
                contracts.stats.protocol_contracts_verified
            );
        } else {
            tracing::info!("Semantic analysis without contract verification");
        }

        // If contracts are available, enable contract-aware type checking
        // This allows the type checker to leverage verified preconditions/postconditions
        // to strengthen type inference and validate call sites
        if let Some(ref contracts) = contracts_opt {
            // Store contract count for metrics
            // Note: Full contract integration with TypeChecker requires adding
            // a contract_context field to TypeChecker (see contract_integration.rs)
            // For now, we log availability and pass through contracts unchanged
            tracing::debug!(
                "Type checker initialized with {} verified contracts available",
                contracts.verified_contracts.len()
            );
        }

        let all_warnings = List::new();

        let semantic_error = |error: anyhow::Error| List::from_iter([
            DiagnosticBuilder::new(Severity::Error).message(format!("{error:#}")).build()
        ]);
        let selected = self.select_owned_sources(modules).map_err(semantic_error)?;
        let registry = self.module_registry.clone()
            .unwrap_or_else(|| self.configured_checker().module_registry().clone());
        crate::owned_source_semantics::register_sources(&registry,
            selected.iter().map(|(owner, path, ast)| (owner.as_str(), path.as_str(), ast)),
        ).map_err(semantic_error)?;
        let mut type_metrics = verum_types::TypeCheckMetrics::new();
        for (owner, _, module) in &selected {
            let mut checker = self.configured_checker();
            crate::owned_source_semantics::check_source_module(
                &mut checker, owner, module, &registry,
            )?;
            type_metrics.synth_count += checker.metrics.synth_count;
            type_metrics.check_count += checker.metrics.check_count;
            type_metrics.unify_count += checker.metrics.unify_count;
            type_metrics.refinement_checks += checker.metrics.refinement_checks;
            type_metrics.protocol_checks += checker.metrics.protocol_checks;
        }

        let duration = start.elapsed();
        let mut metrics = PhaseMetrics::new(self.name()).with_duration(duration);

        // Add type checker metrics
        metrics.add_custom_metric("synth_count", type_metrics.synth_count.to_string());
        metrics.add_custom_metric("check_count", type_metrics.check_count.to_string());
        metrics.add_custom_metric("unify_count", type_metrics.unify_count.to_string());
        metrics.add_custom_metric(
            "refinement_checks",
            type_metrics.refinement_checks.to_string(),
        );
        metrics.add_custom_metric("protocol_checks", type_metrics.protocol_checks.to_string());

        // Add contract metrics if available
        if let Some(ref contracts) = contracts_opt {
            metrics.add_custom_metric(
                "verified_contracts",
                contracts.verified_contracts.len().to_string(),
            );
        }

        tracing::info!(
            "Semantic analysis complete: {} modules, {:.2}ms",
            modules.len(),
            duration.as_millis()
        );
        tracing::debug!("{}", type_metrics.report());

        let selected_modules = selected.into_iter().map(|(_, _, module)| module).collect();
        let data = match input.data {
            PhaseData::AstModulesWithContracts { verification_results, .. } =>
                PhaseData::AstModulesWithContracts { modules: selected_modules, verification_results },
            _ => PhaseData::AstModules(selected_modules),
        };
        Ok(PhaseOutput {
            data,
            warnings: all_warnings,
            metrics,
        })
    }

    fn can_parallelize(&self) -> bool {
        false // Type checking has dependencies
    }

    fn metrics(&self) -> PhaseMetrics {
        PhaseMetrics::new(self.name())
    }
}
