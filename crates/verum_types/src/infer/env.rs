//! Type environment management: stdlib bootstrap, pre-registration, constraint solving.
//!
//! Contains ~205 `TypeChecker` methods covering:
//! - Stdlib constructor registration (`register_stdlib_constructors_from_metadata`)
//! - Pre-registration (`pre_register_module`, `pre_register_module_public`)
//! - Deferred constraint solving (`defer_constraint`, `solve_deferred_constraints`)
//! - Const evaluation (`const_fold_expr`, `const_eval_expr`)
//! - Meta type / builtin registration (`register_meta_types`, `register_meta_builtins`)
//! - Type algebra helpers (`expand_type_alias`, `substitute_with_params`)

#[allow(unused_imports)]
use crate::const_eval::ConstEvaluator;
#[allow(unused_imports)]
use crate::context::{TypeContext, TypeScheme};
#[allow(unused_imports)]
use crate::context_check::{ContextChecker, ContextRequirement, ContextSet};
#[allow(unused_imports)]
use crate::integer_hierarchy::IntegerHierarchy;
#[allow(unused_imports)]
use crate::meta_context::{MetaContextValidation, MetaContextValidator};
#[allow(unused_imports)]
use crate::operator_protocols::{OperatorProtocols, OutputStrategy};
#[allow(unused_imports)]
use crate::protocol::ProtocolChecker;
#[allow(unused_imports)]
use crate::refinement::RefinementChecker;
#[allow(unused_imports)]
use crate::stage_checker::{StageChecker, StageConfig};
#[allow(unused_imports)]
use crate::subtype::Subtyping;
#[allow(unused_imports)]
use crate::ty::{Type, TypeVar};
#[allow(unused_imports)]
use crate::unify::Unifier;
#[allow(unused_imports)]
use crate::{Result, TypeCheckMetrics, TypeError};
#[allow(unused_imports)]
use super::{
    DeferredConstraint, DeferredVerificationGoal, FunctionContract, GeneratorContext,
    GlobalDepthGuard, InferMode, InferResult, TypeChecker,
    WKT_HEAP, WKT_RESULT, WKT_SHARED,
    DEREF_COERCION_DEPTH, GLOBAL_CALL_DEPTH, NORMALIZE_DEPTH,
    AST_TO_TYPE_DEPTH, TYPE_RESOLUTION_STACK, NORMALIZE_TYPE_STACK,
    is_stdlib_toplevel_path, span_to_line_col,
    ConversionPath, ConversionStep, NormalizeTypeCycleGuard, ThreadLocalDepthGuard,
    resolve_builtin_meta_type, type_kind_description,
    read_param_classification, expr_kind_description,
    collect_inline_mount_reexports_recursive, mount_tree_exports_name,
};
#[allow(unused_imports)]
use std::time::Instant;
#[allow(unused_imports)]
use verum_ast::{BinOp, Block, Expr, ExprKind, LiteralKind, Stmt, StmtKind, TokenTree, UnOp, Item};
#[allow(unused_imports)]
use verum_ast::decl::{
    FunctionBody, FunctionDecl, FunctionParamKind, ImplDecl, RecordField, TypeDecl, TypeDeclBody,
    ContextDecl, ProtocolDecl, Visibility, MountDecl, MountTree, MountTreeKind, ResourceModifier,
};
#[allow(unused_imports)]
use verum_ast::pattern::Pattern;
#[allow(unused_imports)]
use verum_ast::span::{Span, Spanned};
#[allow(unused_imports)]
use verum_ast::ty::{Ident, Path};
#[allow(unused_imports)]
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};
#[allow(unused_imports)]
use verum_common::well_known_types::WellKnownType as WKT;
#[allow(unused_imports)]
use verum_common::well_known_types::type_names as wkt_names;
#[allow(unused_imports)]
use verum_common::{Heap, List, Map, Maybe, Set, Shared, Text, ToText};
#[allow(unused_imports)]
use verum_modules::{ModulePath, ModuleRegistry, NameResolver, resolve_import, resolver::NameKind};

impl TypeChecker {
    pub(super) fn register_stdlib_constructors_from_metadata(
        &mut self,
        metadata: &crate::core_metadata::CoreMetadata,
    ) {
        use crate::core_metadata::{TypeDescriptorKind, VariantPayload};
        use crate::ty::{InductiveConstructor, Type};

        // Walk types in source declaration order (stdlib layer order →
        // .vr declaration order). Same rationale as `load_stdlib_from_metadata`:
        // first-registered-wins for inductive constructor lookup, no hardcoded
        // priority list, no compiler-side knowledge of stdlib type names.
        for (type_name, type_desc) in Self::stdlib_iteration_order(metadata) {
            if let TypeDescriptorKind::Variant { cases } = &type_desc.kind {
                let mut constructors = verum_common::List::new();

                for case in cases.iter() {
                    let result_type = if type_desc.generic_params.is_empty() {
                        Type::Named {
                            path: Self::text_to_path(type_name),
                            args: verum_common::List::new(),
                        }
                    } else {
                        Type::Generic {
                            name: type_name.clone(),
                            args: type_desc
                                .generic_params
                                .iter()
                                .map(|_| Type::Var(crate::ty::TypeVar::fresh()))
                                .collect(),
                        }
                    };

                    let constructor = match &case.payload {
                        verum_common::Maybe::None => {
                            InductiveConstructor::unit(case.name.clone(), result_type)
                        }
                        verum_common::Maybe::Some(VariantPayload::Tuple(types)) => {
                            let args: verum_common::List<Type> = types
                                .iter()
                                .map(|t| Type::Named {
                                    path: Self::text_to_path(t),
                                    args: verum_common::List::new(),
                                })
                                .collect();
                            InductiveConstructor::with_args(case.name.clone(), args, result_type)
                        }
                        verum_common::Maybe::Some(VariantPayload::Record(fields)) => {
                            let args: verum_common::List<Type> = fields
                                .iter()
                                .map(|f| Type::Named {
                                    path: Self::text_to_path(&f.ty),
                                    args: verum_common::List::new(),
                                })
                                .collect();
                            // Register variant record fields as __struct_fields_<VariantName>
                            // so pattern matching `Rect { w, h }` can resolve field types.
                            let struct_key = format!("__struct_fields_{}", case.name);
                            let mut field_map = indexmap::IndexMap::new();
                            for f in fields.iter() {
                                let field_ty = Type::Named {
                                    path: Self::text_to_path(&f.ty),
                                    args: verum_common::List::new(),
                                };
                                field_map
                                    .insert(verum_common::Text::from(f.name.as_str()), field_ty);
                            }
                            self.ctx.define_type(struct_key, Type::Record(field_map));
                            InductiveConstructor::with_args(case.name.clone(), args, result_type)
                        }
                    };

                    constructors.push(constructor);
                }

                self.ctx
                    .register_inductive_type(type_name.clone(), constructors);
            }
        }

        // Always register Bool constructors (primitive)
        let bool_constructors = verum_common::List::from_iter([
            InductiveConstructor::unit(verum_common::Text::from("true"), Type::Bool),
            InductiveConstructor::unit(verum_common::Text::from("false"), Type::Bool),
        ]);
        self.ctx.register_inductive_type("Bool", bool_constructors);

        // Register Unit constructor
        let unit_constructors = verum_common::List::from_iter([InductiveConstructor::unit(
            verum_common::Text::from("()"),
            Type::Unit,
        )]);
        self.ctx.register_inductive_type("Unit", unit_constructors);
    }

    /// Get the stdlib metadata (if loaded).
    /// Stdlib metadata is always loaded for user code compilation.
    pub fn core_metadata(&self) -> Option<&crate::core_metadata::CoreMetadata> {
        match &self.core_metadata {
            Maybe::Some(arc) => Some(arc.as_ref()),
            Maybe::None => None,
        }
    }

    /// Set the stdlib module loader for lazy loading.
    ///

    /// When set, the TypeChecker will use this loader to load stdlib modules
    /// on-demand when they are first imported, rather than requiring all
    /// modules to be pre-loaded.
    ///

    /// # Arguments
    ///

    /// * `resolver` - The module resolver implementation
    pub fn set_lazy_resolver(&mut self, resolver: verum_modules::SharedModuleResolver) {
        self.lazy_resolver = Some(resolver);
    }

    /// Get the lazy module resolver (if set).
    pub fn lazy_resolver(&self) -> Option<&verum_modules::SharedModuleResolver> {
        self.lazy_resolver.as_ref()
    }

    /// Get the operator protocols mapping
    /// Stdlib-agnostic type system: type checker operates without hardcoded knowledge of stdlib types, stdlib types registered from parsed .vr files
    pub fn operator_protocols(&self) -> &OperatorProtocols {
        &self.operator_protocols
    }

    /// Get mutable access to operator protocols for customization
    /// Stdlib-agnostic type system: type checker operates without hardcoded knowledge of stdlib types, stdlib types registered from parsed .vr files
    pub fn operator_protocols_mut(&mut self) -> &mut OperatorProtocols {
        &mut self.operator_protocols
    }

    /// Get mutable access to the unifier for registering stdlib types.
    pub fn unifier_mut(&mut self) -> &mut Unifier {
        &mut self.unifier
    }

    /// Set the current cog name for orphan-rule checking. Without a
    /// current cog, ProtocolChecker::check_orphan_rule returns Ok(())
    /// unconditionally — which silently permits orphan impls in user
    /// code. Pipelines must call this early (before register_impl).
    pub fn set_current_cog(&mut self, cog_name: impl Into<verum_common::Text>) {
        let name: verum_common::Text = cog_name.into();
        // Two consumers: (1) ProtocolChecker uses this for orphan-rule
        // discipline; (2) `ImportOrigin::classify` uses this to tell
        // project paths apart from stdlib/external during glob
        // shadow arbitration (#146 / MOD-MED-2).
        self.current_cog_name = name.clone();
        self.protocol_checker.write().set_current_crate(name);
    }

    /// Create a new type checker that owns a [`SharedModuleRegistry`].
    ///

    /// **Recommended** entry point for new code. The newtype encapsulates the
    /// `Shared<RwLock<...>>` wrapping so callers cannot accidentally pass the
    /// wrong shape (a class of bug that historically produced 30+ `mismatched
    /// types` errors when the registry's wrapping was tightened — see
    /// [`verum_modules::SharedModuleRegistry`] for context).
    pub fn with_shared_registry(registry: verum_modules::SharedModuleRegistry) -> Self {
        Self::with_registry(registry.into_inner())
    }

    /// Create a new type checker with a shared module registry
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — Shared module state
    ///

    /// Legacy raw-handle API kept for callers that still hold an unwrapped
    /// `Shared<RwLock<ModuleRegistry>>`. New code should prefer
    /// [`Self::with_shared_registry`].
    pub fn with_registry(registry: Shared<parking_lot::RwLock<ModuleRegistry>>) -> Self {
        Self {
            ctx: TypeContext::new(),
            unifier: Unifier::new(),
            refinement: RefinementChecker::new(Default::default()),
            subtyping: Subtyping::new(),
            const_eval: ConstEvaluator::new(),
            protocol_checker: Shared::new(parking_lot::RwLock::new(ProtocolChecker::new())),
            integer_hierarchy: IntegerHierarchy::new(),
            kind_inferer: crate::kind_inference::KindInferer::new_minimal(),
            metrics: TypeCheckMetrics::new(),
            generator_context: Maybe::None,
            diagnostics: List::new(),
            deferred_soundness_errors: Vec::new(),
            glob_import_provenance: std::collections::HashMap::new(),
            current_cog_name: verum_common::Text::from(""),
            dependent_enabled: true,
            higher_kinded_enabled: true,
            universe_poly_enabled: false,
            coinductive_enabled: true,
            quotient_enabled: true,
            instance_search_enabled: true,
            coherence_check_depth: 16,
            higher_kinded_protocols_enabled: false,
            generic_associated_types_enabled: false,
            classification_map: std::collections::HashMap::new(),
            module_resolver: NameResolver::new(),
            module_registry: registry,
            current_module_path: verum_common::Text::from("cog"),
            inline_modules: Map::new(),
            module_aliases: Map::new(),
            preregistered_modules: std::collections::HashSet::new(),
            blanket_impls_registered_modules: std::collections::HashSet::new(),
            primitive_impls_registered_modules: std::collections::HashSet::new(),
            module_context: Maybe::None,
            current_function_return_type: Maybe::None,
            current_function_name: Maybe::None,
            current_function_return_span: Maybe::None,
            current_function_params: Set::new(),
            type_registry: crate::type_registry::TypeRegistry::new(),
            context_resolver: crate::context_resolution::ContextResolver::new(),
            current_self_type: Maybe::None,
            capability_checker: crate::capability::CapabilityChecker::new(),
            context_declarations: Map::new(),
            method_capability_mapper: crate::capability::MethodCapabilityMapper::new(),
            inherent_methods: Shared::new(parking_lot::RwLock::new(Map::new())),
            method_impl_patterns: Shared::new(parking_lot::RwLock::new(Map::new())),
            variance_checker: crate::variance::VarianceChecker::new(),
            variant_type_names: Map::new(),
            variant_collision_log: List::new(),
            variant_record_fields: Map::new(),
            variant_constructor_parents: Map::new(),
            type_var_bounds: Map::new(),
            hkt_type_var_by_name: Map::new(),
            type_var_type_bounds: Map::new(),
            deferred_constraints: List::new(),
            property_inferrer: crate::computational_properties::PropertyInferrer::new(),
            in_async_context: false,
            try_recover_depth: 0,
            current_function_throws: Maybe::None,
            context_checker: ContextChecker::new(),
            stage_checker: StageChecker::with_defaults(),
            current_function_stage: 0,
            current_function_is_transparent: false,
            current_function_contexts: Maybe::None,
            current_function_call_sites: Map::new(),
            current_function_being_checked: Maybe::None,
            init_tracker: crate::context::InitTracker::new(),
            affine_tracker: crate::affine::AffineTracker::with_core(),
            borrow_tracker: crate::aliasing::BorrowTracker::new(),
            in_call_arg_context: false,
            self_by_value_methods: std::collections::HashSet::new(),
            in_unsafe_context: false,
            in_impl_block: false,
            types_being_registered: std::collections::HashSet::new(),
            imports_in_progress: std::collections::HashSet::new(),
            glob_imports_in_progress: std::collections::HashSet::new(),
            glob_imports_stack: Vec::new(),
            reexport_resolution_in_progress: std::collections::HashSet::new(),
            imported_names: Map::new(),
            constants_being_evaluated: std::collections::HashSet::new(),
            constant_dependencies: Map::new(),
            current_constant_path: Maybe::None,
            imported_constant_paths: Map::new(),
            termination_checker: crate::termination::TerminationChecker::new(),
            function_required_params: Map::new(),
            function_contracts: Map::new(),
            function_param_names: Map::new(),
            function_param_classifications: Map::new(),
            core_metadata: Maybe::None,
            lazy_resolver: None,
            session_registry: None,
            operator_protocols: OperatorProtocols::standard(),
            type_generics_count: Map::new(),
            refinement_evidence: crate::refinement_evidence::RefinementEvidence::new(),
            prototype_mode: false,
            implicit_context: crate::implicit::ImplicitContext::new(),
            mutable_bindings: std::collections::HashSet::new(),
            pattern_declarations: Map::new(),
            hit_path_constructors: Map::new(),
            deferred_verification_goals: List::new(),
            stdlib_single_file_mode: false,
            user_code_phase: false,
            explicit_imports: std::collections::HashSet::new(),
            in_explicit_import_registration: false,
            skolem_tracker: crate::existential::SkolemTracker::new(),
            cfg_evaluator: verum_ast::cfg::CfgEvaluator::new(),
            inference_depth: Cell::new(0),
        }
    }

    /// Create a TypeChecker with a shared inherent_methods map
    ///

    /// This enables order-independent method resolution by sharing
    /// the methods map across multiple TypeChecker instances.
    /// Methods registered in one module become immediately visible
    /// to all other modules sharing the same map.
    ///

    /// Order-independent declarations: types and functions can be referenced before their definition within a module
    pub fn with_shared_methods(
        inherent_methods: Shared<
            parking_lot::RwLock<Map<Text, Map<Text, crate::context::TypeScheme>>>,
        >,
    ) -> Self {
        Self {
            ctx: TypeContext::new(),
            unifier: Unifier::new(),
            refinement: RefinementChecker::new(Default::default()),
            subtyping: Subtyping::new(),
            const_eval: ConstEvaluator::new(),
            protocol_checker: Shared::new(parking_lot::RwLock::new(ProtocolChecker::new())),
            integer_hierarchy: IntegerHierarchy::new(),
            kind_inferer: crate::kind_inference::KindInferer::new_minimal(),
            metrics: TypeCheckMetrics::new(),
            generator_context: Maybe::None,
            diagnostics: List::new(),
            deferred_soundness_errors: Vec::new(),
            glob_import_provenance: std::collections::HashMap::new(),
            current_cog_name: verum_common::Text::from(""),
            dependent_enabled: true,
            higher_kinded_enabled: true,
            universe_poly_enabled: false,
            coinductive_enabled: true,
            quotient_enabled: true,
            instance_search_enabled: true,
            coherence_check_depth: 16,
            higher_kinded_protocols_enabled: false,
            generic_associated_types_enabled: false,
            classification_map: std::collections::HashMap::new(),
            module_resolver: NameResolver::new(),
            module_registry: Shared::new(parking_lot::RwLock::new(ModuleRegistry::new())),
            current_module_path: verum_common::Text::from("cog"),
            inline_modules: Map::new(),
            module_aliases: Map::new(),
            preregistered_modules: std::collections::HashSet::new(),
            blanket_impls_registered_modules: std::collections::HashSet::new(),
            primitive_impls_registered_modules: std::collections::HashSet::new(),
            module_context: Maybe::None,
            current_function_return_type: Maybe::None,
            current_function_name: Maybe::None,
            current_function_return_span: Maybe::None,
            current_function_params: Set::new(),
            type_registry: crate::type_registry::TypeRegistry::new(),
            context_resolver: crate::context_resolution::ContextResolver::new(),
            current_self_type: Maybe::None,
            capability_checker: crate::capability::CapabilityChecker::new(),
            context_declarations: Map::new(),
            method_capability_mapper: crate::capability::MethodCapabilityMapper::new(),
            inherent_methods,
            method_impl_patterns: Shared::new(parking_lot::RwLock::new(Map::new())),
            variance_checker: crate::variance::VarianceChecker::new(),
            variant_type_names: Map::new(),
            variant_collision_log: List::new(),
            variant_record_fields: Map::new(),
            variant_constructor_parents: Map::new(),
            type_var_bounds: Map::new(),
            hkt_type_var_by_name: Map::new(),
            type_var_type_bounds: Map::new(),
            deferred_constraints: List::new(),
            property_inferrer: crate::computational_properties::PropertyInferrer::new(),
            in_async_context: false,
            try_recover_depth: 0,
            current_function_throws: Maybe::None,
            context_checker: ContextChecker::new(),
            stage_checker: StageChecker::with_defaults(),
            current_function_stage: 0,
            current_function_is_transparent: false,
            current_function_contexts: Maybe::None,
            current_function_call_sites: Map::new(),
            current_function_being_checked: Maybe::None,
            init_tracker: crate::context::InitTracker::new(),
            affine_tracker: crate::affine::AffineTracker::with_core(),
            borrow_tracker: crate::aliasing::BorrowTracker::new(),
            in_call_arg_context: false,
            self_by_value_methods: std::collections::HashSet::new(),
            in_unsafe_context: false,
            in_impl_block: false,
            types_being_registered: std::collections::HashSet::new(),
            imports_in_progress: std::collections::HashSet::new(),
            glob_imports_in_progress: std::collections::HashSet::new(),
            glob_imports_stack: Vec::new(),
            reexport_resolution_in_progress: std::collections::HashSet::new(),
            imported_names: Map::new(),
            constants_being_evaluated: std::collections::HashSet::new(),
            constant_dependencies: Map::new(),
            current_constant_path: Maybe::None,
            imported_constant_paths: Map::new(),
            termination_checker: crate::termination::TerminationChecker::new(),
            function_required_params: Map::new(),
            function_contracts: Map::new(),
            function_param_names: Map::new(),
            function_param_classifications: Map::new(),
            core_metadata: Maybe::None,
            lazy_resolver: None,
            session_registry: None,
            operator_protocols: OperatorProtocols::standard(),
            type_generics_count: Map::new(),
            refinement_evidence: crate::refinement_evidence::RefinementEvidence::new(),
            prototype_mode: false,
            implicit_context: crate::implicit::ImplicitContext::new(),
            mutable_bindings: std::collections::HashSet::new(),
            pattern_declarations: Map::new(),
            hit_path_constructors: Map::new(),
            deferred_verification_goals: List::new(),
            stdlib_single_file_mode: false,
            user_code_phase: false,
            explicit_imports: std::collections::HashSet::new(),
            in_explicit_import_registration: false,
            skolem_tracker: crate::existential::SkolemTracker::new(),
            cfg_evaluator: verum_ast::cfg::CfgEvaluator::new(),
            inference_depth: Cell::new(0),
        }
    }

    /// Create a new TypeChecker with shared inherent methods AND a pre-populated ProtocolChecker.
    ///

    /// This enables stdlib protocol implementations to be shared across all modules.
    /// The ProtocolChecker should be pre-populated with stdlib impls before user code
    /// is processed, allowing IntoIterator, Future, and other protocol resolutions
    /// to work without hardcoded type knowledge.
    ///

    /// Stdlib-agnostic type system: type checker operates without hardcoded knowledge of stdlib types, stdlib types registered from parsed .vr files
    pub fn with_shared_methods_and_protocols(
        inherent_methods: Shared<
            parking_lot::RwLock<Map<Text, Map<Text, crate::context::TypeScheme>>>,
        >,
        protocol_checker: Shared<parking_lot::RwLock<ProtocolChecker>>,
    ) -> Self {
        Self {
            ctx: TypeContext::new(),
            unifier: Unifier::new(),
            refinement: RefinementChecker::new(Default::default()),
            subtyping: Subtyping::new(),
            const_eval: ConstEvaluator::new(),
            protocol_checker,
            integer_hierarchy: IntegerHierarchy::new(),
            kind_inferer: crate::kind_inference::KindInferer::new_minimal(),
            metrics: TypeCheckMetrics::new(),
            generator_context: Maybe::None,
            diagnostics: List::new(),
            deferred_soundness_errors: Vec::new(),
            glob_import_provenance: std::collections::HashMap::new(),
            current_cog_name: verum_common::Text::from(""),
            dependent_enabled: true,
            higher_kinded_enabled: true,
            universe_poly_enabled: false,
            coinductive_enabled: true,
            quotient_enabled: true,
            instance_search_enabled: true,
            coherence_check_depth: 16,
            higher_kinded_protocols_enabled: false,
            generic_associated_types_enabled: false,
            classification_map: std::collections::HashMap::new(),
            module_resolver: NameResolver::new(),
            module_registry: Shared::new(parking_lot::RwLock::new(ModuleRegistry::new())),
            current_module_path: verum_common::Text::from("cog"),
            inline_modules: Map::new(),
            module_aliases: Map::new(),
            preregistered_modules: std::collections::HashSet::new(),
            blanket_impls_registered_modules: std::collections::HashSet::new(),
            primitive_impls_registered_modules: std::collections::HashSet::new(),
            module_context: Maybe::None,
            current_function_return_type: Maybe::None,
            current_function_name: Maybe::None,
            current_function_return_span: Maybe::None,
            current_function_params: Set::new(),
            type_registry: crate::type_registry::TypeRegistry::new(),
            context_resolver: crate::context_resolution::ContextResolver::new(),
            current_self_type: Maybe::None,
            capability_checker: crate::capability::CapabilityChecker::new(),
            context_declarations: Map::new(),
            method_capability_mapper: crate::capability::MethodCapabilityMapper::new(),
            inherent_methods,
            method_impl_patterns: Shared::new(parking_lot::RwLock::new(Map::new())),
            variance_checker: crate::variance::VarianceChecker::new(),
            variant_type_names: Map::new(),
            variant_collision_log: List::new(),
            variant_record_fields: Map::new(),
            variant_constructor_parents: Map::new(),
            type_var_bounds: Map::new(),
            hkt_type_var_by_name: Map::new(),
            type_var_type_bounds: Map::new(),
            deferred_constraints: List::new(),
            property_inferrer: crate::computational_properties::PropertyInferrer::new(),
            in_async_context: false,
            try_recover_depth: 0,
            current_function_throws: Maybe::None,
            context_checker: ContextChecker::new(),
            stage_checker: StageChecker::with_defaults(),
            current_function_stage: 0,
            current_function_is_transparent: false,
            current_function_contexts: Maybe::None,
            current_function_call_sites: Map::new(),
            current_function_being_checked: Maybe::None,
            init_tracker: crate::context::InitTracker::new(),
            affine_tracker: crate::affine::AffineTracker::with_core(),
            borrow_tracker: crate::aliasing::BorrowTracker::new(),
            in_call_arg_context: false,
            self_by_value_methods: std::collections::HashSet::new(),
            in_unsafe_context: false,
            in_impl_block: false,
            types_being_registered: std::collections::HashSet::new(),
            imports_in_progress: std::collections::HashSet::new(),
            glob_imports_in_progress: std::collections::HashSet::new(),
            glob_imports_stack: Vec::new(),
            reexport_resolution_in_progress: std::collections::HashSet::new(),
            imported_names: Map::new(),
            constants_being_evaluated: std::collections::HashSet::new(),
            constant_dependencies: Map::new(),
            current_constant_path: Maybe::None,
            imported_constant_paths: Map::new(),
            termination_checker: crate::termination::TerminationChecker::new(),
            function_required_params: Map::new(),
            function_contracts: Map::new(),
            function_param_names: Map::new(),
            function_param_classifications: Map::new(),
            core_metadata: Maybe::None,
            lazy_resolver: None,
            session_registry: None,
            operator_protocols: OperatorProtocols::standard(),
            type_generics_count: Map::new(),
            refinement_evidence: crate::refinement_evidence::RefinementEvidence::new(),
            prototype_mode: false,
            implicit_context: crate::implicit::ImplicitContext::new(),
            mutable_bindings: std::collections::HashSet::new(),
            pattern_declarations: Map::new(),
            hit_path_constructors: Map::new(),
            deferred_verification_goals: List::new(),
            stdlib_single_file_mode: false,
            user_code_phase: false,
            explicit_imports: std::collections::HashSet::new(),
            in_explicit_import_registration: false,
            skolem_tracker: crate::existential::SkolemTracker::new(),
            cfg_evaluator: verum_ast::cfg::CfgEvaluator::new(),
            inference_depth: Cell::new(0),
        }
    }

    /// Get the shared inherent_methods map for cross-module sharing
    pub fn get_inherent_methods(
        &self,
    ) -> Shared<parking_lot::RwLock<Map<Text, Map<Text, crate::context::TypeScheme>>>> {
        self.inherent_methods.clone()
    }

    /// Set current_self_type and synchronize with the unifier's self_type.
    /// This ensures that both ast_to_type resolution and unification properly
    /// handle the Self type within implement blocks.
    fn set_current_self_type(&mut self, self_type: Maybe<Type>) {
        match &self_type {
            Maybe::Some(ty) => self.unifier.set_self_type(Some(ty.clone())),
            Maybe::None => self.unifier.set_self_type(None),
        }
        self.current_self_type = self_type;
    }

    // ==================== Prototype Mode Methods ====================
    // @prototype mode: relaxed type checking for rapid prototyping, deferred refinement verification — @prototype Mode

    /// Enable prototype mode (relaxed type checking).
    /// Certain type errors become warnings in this mode.
    pub fn enable_prototype_mode(&mut self) {
        self.prototype_mode = true;
    }

    /// Disable prototype mode (strict type checking).
    pub fn disable_prototype_mode(&mut self) {
        self.prototype_mode = false;
    }

    /// Check if prototype mode is enabled.
    pub fn is_prototype_mode(&self) -> bool {
        self.prototype_mode
    }

    /// Check if a function has the @prototype attribute.
    /// Used to automatically enable prototype mode when checking such functions.
    pub fn has_prototype_attribute(attrs: &[verum_ast::attr::Attribute]) -> bool {
        attrs.iter().any(|attr| attr.name.as_str() == "prototype")
    }

    // ==================== Testing Support Methods ====================
    // These methods are for unit testing affine type checking

    /// Register an affine type for testing purposes
    /// Spec: L0-critical/reference_system/value_transfer - Affine type safety
    pub fn register_affine_type_for_testing(&mut self, type_name: &str) {
        self.affine_tracker.register_affine_type(type_name);
    }

    /// Register a type in the context for testing purposes
    pub fn register_type_for_testing(&mut self, name: &str, ty: Type) {
        self.ctx.define_type(verum_common::Text::from(name), ty);
    }

    /// Check a path expression for affine usage (for testing)
    /// Returns Ok if the value can be used, Err if already moved
    pub fn check_path_for_affine(&mut self, path: &verum_ast::ty::Path, span: Span) -> Result<()> {
        if path.segments.len() == 1 {
            if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                return self.affine_tracker.use_value(id.name.as_str(), span);
            }
        }
        Ok(())
    }

    /// Check if a type name is registered as affine (for testing)
    pub fn is_type_affine_by_name(&self, type_name: &str) -> bool {
        self.affine_tracker.is_affine_type(type_name)
    }

    /// Check if a type contains any affine types (for affine contagion)
    /// Types containing affine types are also treated as affine.
    /// Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — #affine-types
    pub(crate) fn type_contains_affine(&self, ty: &Type) -> bool {
        self.type_contains_affine_impl(ty, &mut std::collections::HashSet::new())
    }

    fn type_contains_affine_impl(
        &self,
        ty: &Type,
        visited: &mut std::collections::HashSet<Text>,
    ) -> bool {
        match ty {
            Type::Named { path, args } => {
                // Check if the named type is affine
                if let Some(seg) = path.segments.last() {
                    if let verum_ast::ty::PathSegment::Name(ident) = seg {
                        let name = ident.name.as_str();
                        if self.affine_tracker.is_affine_type(name) {
                            return true;
                        }
                        // Recursively check the type definition if not yet visited
                        let name_text: Text = name.into();
                        if !visited.contains(&name_text) {
                            visited.insert(name_text.clone());
                            // First, try looking up the struct fields (for record types)
                            let struct_key = format!("__struct_fields_{}", name);
                            if let Option::Some(fields_ty) = self.ctx.lookup_type(&struct_key) {
                                if self.type_contains_affine_impl(fields_ty, visited) {
                                    return true;
                                }
                            }
                            // Fall back to looking up the type directly (for aliases and variants)
                            if let Option::Some(def_ty) = self.ctx.lookup_type(name) {
                                if self.type_contains_affine_impl(def_ty, visited) {
                                    return true;
                                }
                            }
                        }
                    }
                }
                // AFFINE CONTAGION: Check type arguments for affine types
                // e.g., Wrapper<Handle> where Handle is affine -> Wrapper<Handle> contains affine
                // Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — #affine-contagion
                args.iter()
                    .any(|arg| self.type_contains_affine_impl(arg, visited))
            }
            Type::Generic { name, args } => {
                // Check if the base type is affine
                if self.affine_tracker.is_affine_type(name.as_str()) {
                    return true;
                }
                // Recursively check the type definition
                let name_text: Text = name.clone();
                if !visited.contains(&name_text) {
                    visited.insert(name_text.clone());
                    if let Option::Some(def_ty) = self.ctx.lookup_type(name.as_str()) {
                        if self.type_contains_affine_impl(def_ty, visited) {
                            return true;
                        }
                    }
                }
                // Check if any type argument contains affine types
                args.iter()
                    .any(|arg| self.type_contains_affine_impl(arg, visited))
            }
            Type::Tuple(types) => types
                .iter()
                .any(|t| self.type_contains_affine_impl(t, visited)),
            Type::Record(fields) => fields
                .values()
                .any(|t| self.type_contains_affine_impl(t, visited)),
            Type::Variant(variants) => variants
                .values()
                .any(|t| self.type_contains_affine_impl(t, visited)),
            Type::Array { element, .. } => self.type_contains_affine_impl(element, visited),
            Type::Slice { element } => self.type_contains_affine_impl(element, visited),
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => self.type_contains_affine_impl(inner, visited),
            _ => false,
        }
    }

    /// Lookup a type in context (for testing/debugging)
    pub fn lookup_type_for_testing(&self, name: &str) -> Option<Type> {
        self.ctx.lookup_type(name).cloned()
    }

    /// Debug method: Check if a qualified name (like "File.open") is in the environment
    pub fn lookup_qualified_name_for_testing(&self, qualified_name: &str) -> Option<String> {
        self.ctx
            .env
            .lookup(qualified_name)
            .map(|scheme| format!("{:?}", scheme))
    }

    /// Debug method: Check if a static method is in inherent_methods
    pub fn lookup_static_method_for_testing(
        &self,
        type_name: &str,
        method_name: &str,
    ) -> Option<String> {
        let methods_guard = self.inherent_methods.read();
        let type_name_text = verum_common::Text::from(type_name);
        let static_key = verum_common::Text::from(format!("$static${}", method_name));
        methods_guard
            .get(&type_name_text)
            .and_then(|methods| methods.get(&static_key).cloned())
            .map(|scheme| format!("{:?}", scheme))
    }

    /// Debug method: Check if a method exists in inherent_methods (non-static)
    pub fn lookup_instance_method_for_testing(
        &self,
        type_name: &str,
        method_name: &str,
    ) -> Option<String> {
        let methods_guard = self.inherent_methods.read();
        let type_name_text = verum_common::Text::from(type_name);
        let method_name_text = verum_common::Text::from(method_name);
        methods_guard
            .get(&type_name_text)
            .and_then(|methods| methods.get(&method_name_text).cloned())
            .map(|scheme| format!("{:?}", scheme))
    }

    /// Debug method: Look up protocol method via protocol_checker
    pub fn lookup_protocol_method_for_testing(
        &self,
        type_name: &str,
        method_name: &str,
    ) -> Option<String> {
        let ty = Type::Named {
            path: verum_ast::ty::Path::single(verum_ast::Ident::new(type_name, Span::default())),
            args: List::new(),
        };
        let method_text = verum_common::Text::from(method_name);
        match self
            .protocol_checker
            .read()
            .lookup_protocol_method(&ty, &method_text)
        {
            Ok(Maybe::Some(method_ty)) => Some(format!("{:?}", method_ty)),
            _ => None,
        }
    }

    /// Debug: List all methods in inherent_methods for a type
    pub fn list_methods_for_type(&self, type_name: &str) -> Vec<String> {
        let methods_guard = self.inherent_methods.read();
        let type_name_text = verum_common::Text::from(type_name);
        methods_guard
            .get(&type_name_text)
            .map(|methods| methods.keys().map(|k| k.to_string()).collect())
            .unwrap_or_default()
    }

    /// Per-instantiation impl gate.
    ///

    /// When `method_impl_patterns` has one or more registered patterns
    /// for `(type_name, method_name)`, at least one pattern must match
    /// the receiver's concrete type arguments. Otherwise the method
    /// is considered *not applicable* to this receiver — e.g.
    /// `Register<UInt32, ReadOnly>.write(…)` must be rejected because
    /// `write` is only registered for `Register<T, WriteOnly>` and
    /// `Register<T, ReadWrite>`.
    ///

    /// Returns `true` when:
    ///  - no patterns are registered (backward-compat — primitive
    ///  types like Int/Text have no patterns), OR
    ///  - at least one pattern is compatible with `receiver_args`.
    ///

    /// A pattern slot of `Type::Var(_)` (impl-level generic, e.g. `T`
    /// in `implement<T: Copy> Register<T, ReadOnly>`) matches any
    /// receiver arg. A concrete pattern slot (`Named`, `Generic`, …)
    /// must structurally match the receiver slot.
    pub fn inherent_method_pattern_allows(
        &self,
        type_name: &verum_common::Text,
        method_name: &verum_common::Text,
        receiver_args: &[Type],
    ) -> bool {
        let patterns_guard = self.method_impl_patterns.read();
        let Some(method_patterns) = patterns_guard
            .get(type_name)
            .and_then(|m| m.get(method_name))
        else {
            return true; // no patterns — permissive
        };
        if method_patterns.is_empty() {
            return true;
        }
        // At least one registered impl pattern must accept these args.
        method_patterns
            .iter()
            .any(|pattern| self.type_args_match_impl_pattern(receiver_args, pattern))
    }

    /// Match receiver type args against an impl's self-type arg
    /// pattern. Slot-by-slot; lengths must agree.
    fn type_args_match_impl_pattern(
        &self,
        receiver_args: &[Type],
        pattern: &verum_common::List<Type>,
    ) -> bool {
        if receiver_args.len() != pattern.len() {
            return false;
        }
        receiver_args
            .iter()
            .zip(pattern.iter())
            .all(|(recv, pat)| Self::impl_pattern_slot_matches(recv, pat))
    }

    /// Slot-level match: `Var` pattern slots accept anything; a
    /// concrete slot must structurally match the receiver.
    fn impl_pattern_slot_matches(receiver: &Type, pattern: &Type) -> bool {
        match pattern {
            // Impl-level generic (e.g. `T` in `implement<T: Copy> Register<T, …>`).
            Type::Var(_) | Type::Placeholder { .. } | Type::Unknown => true,
            Type::Named { path: pp, .. } => match receiver {
                Type::Named { path: rp, .. } => {
                    Self::get_protocol_name_str(pp) == Self::get_protocol_name_str(rp)
                }
                Type::Generic { name, .. } => name.as_str() == Self::get_protocol_name_str(pp),
                // A bare TypeVar receiver — nothing is known, stay
                // permissive so inference isn't pinned prematurely.
                Type::Var(_) | Type::Placeholder { .. } | Type::Unknown => true,
                _ => false,
            },
            Type::Generic { name: pn, .. } => match receiver {
                Type::Generic { name: rn, .. } => pn == rn,
                Type::Named { path, .. } => pn.as_str() == Self::get_protocol_name_str(path),
                Type::Var(_) | Type::Placeholder { .. } | Type::Unknown => true,
                _ => false,
            },
            // Primitive literal slot — must equal the receiver primitive.
            Type::Int | Type::Float | Type::Bool | Type::Text | Type::Char | Type::Unit => {
                receiver == pattern
            }
            // Compound types are rare in impl headers; be permissive.
            _ => true,
        }
    }

    // ==================== Definite Assignment Analysis ====================
    // Spec: L0-critical/memory-safety/uninitialized - Compile-time partial init detection

    /// Register a variable as uninitialized with appropriate tracking based on its type.
    ///

    /// For compound types (tuples, arrays, records), creates partial initialization
    /// tracking so we can detect field-by-field or element-by-element initialization.
    fn register_uninitialized_var(&mut self, var_name: &str, ty: &Type) {
        use verum_common::Set;

        match ty {
            Type::Tuple(element_types) => {
                // Track tuple with its size for element-by-element initialization
                self.init_tracker
                    .register_tuple(var_name, element_types.len());
            }
            Type::Array {
                size: Some(len), ..
            } => {
                // Track fixed-size array for element-by-element initialization
                self.init_tracker.register_array(var_name, *len);
            }
            Type::Record(fields) => {
                // Track record with required fields
                let required: Set<Text> = fields.keys().cloned().collect();
                self.init_tracker.register_record(var_name, required);
            }
            Type::Named { path, .. } => {
                // For named types, look up their underlying structure
                let type_name = self.path_to_string(path);
                let struct_key = format!("__struct_fields_{}", type_name);

                // Try to find the fields of the named record type
                if let Option::Some(Type::Record(fields)) =
                    self.ctx.lookup_type(struct_key.as_str())
                {
                    let required: Set<Text> = fields.keys().cloned().collect();
                    self.init_tracker.register_record(var_name, required);
                } else {
                    // Unknown named type - register as simple uninitialized
                    self.init_tracker.register_uninitialized(var_name);
                }
            }
            _ => {
                // Simple types (Int, Text, etc.) - track as simple uninitialized
                self.init_tracker.register_uninitialized(var_name);
            }
        }
    }

    /// Check if a variable can be used (is fully initialized).
    /// Returns an error if the variable is uninitialized or partially initialized.
    fn check_variable_initialized(&self, var_name: &Text, span: Span) -> Result<()> {
        use crate::context::InitState;

        match self.init_tracker.get_state(var_name) {
            Option::Some(InitState::Uninitialized) => Err(TypeError::UseOfUninitializedVariable {
                name: var_name.clone(),
                span,
            }),
            Option::Some(InitState::PartiallyInitialized(partial)) => {
                // Get missing elements for error message
                let missing = match partial {
                    crate::context::PartialInit::Tuple { initialized, total } => {
                        let missing_indices: List<usize> =
                            (0..*total).filter(|i| !initialized.contains(i)).collect();
                        format!("tuple elements {:?}", missing_indices)
                    }
                    crate::context::PartialInit::Array { initialized, len } => {
                        let missing_indices: List<usize> =
                            (0..*len).filter(|i| !initialized.contains(i)).collect();
                        format!("array elements {:?}", missing_indices)
                    }
                    crate::context::PartialInit::Record {
                        initialized,
                        required,
                    } => {
                        let missing_fields: List<verum_common::Text> =
                            required.difference(initialized).cloned().collect();
                        format!("fields {:?}", missing_fields)
                    }
                };
                Err(TypeError::PartiallyInitializedVariable {
                    name: var_name.clone(),
                    missing: missing.into(),
                    span,
                })
            }
            Option::Some(InitState::FullyInitialized) => Ok(()),
            Option::None => Ok(()), // Variable not tracked - assume OK
        }
    }

    /// Check if a specific field of a variable is initialized.
    fn check_field_initialized(&self, var_name: &Text, field: &Text, span: Span) -> Result<()> {
        if !self.init_tracker.is_field_initialized(var_name, field) {
            Err(TypeError::UninitializedField {
                var: var_name.clone(),
                field: field.clone(),
                span,
            })
        } else {
            Ok(())
        }
    }

    /// Track access to an affine field (partial move tracking).
    ///

    /// When accessing a field that has an affine type, this marks the field as moved.
    /// After a field is moved, the parent struct cannot be used as a whole.
    /// Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — #affine-partial-move
    fn track_affine_field_access(
        &mut self,
        var_name: &str,
        field_name: &str,
        field_ty: &Type,
        span: Span,
    ) -> Result<()> {
        // Check if the field type is affine or contains affine types
        // Use type_contains_affine which does a deep check with cycle detection
        if self.affine_tracker.is_type_affine(field_ty) || self.type_contains_affine(field_ty) {
            self.affine_tracker
                .use_field_value(var_name, field_name, span)?;
        }
        Ok(())
    }

    /// Check if a specific index of a variable (tuple or array) is initialized.
    fn check_index_initialized(
        &self,
        var_name: &Text,
        index: usize,
        span: Span,
        is_tuple: bool,
    ) -> Result<()> {
        if !self.init_tracker.is_index_initialized(var_name, index) {
            if is_tuple {
                Err(TypeError::UninitializedTupleElement {
                    var: var_name.clone(),
                    index,
                    span,
                })
            } else {
                Err(TypeError::UninitializedArrayElement {
                    var: var_name.clone(),
                    index,
                    span,
                })
            }
        } else {
            Ok(())
        }
    }

    /// Handle assignment to a variable or its components.
    /// Updates the initialization state based on the assignment target.
    fn handle_assignment(&mut self, left: &Expr, span: Span) {
        use verum_ast::expr::ExprKind;

        match &left.kind {
            // Simple variable assignment: x = value
            ExprKind::Path(path) if path.segments.len() == 1 => {
                if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                    let var_name = verum_common::Text::from(id.name.as_str());
                    self.init_tracker.mark_fully_initialized(&var_name);
                }
            }

            // Field assignment: x.field = value
            ExprKind::Field { expr: obj, field } => {
                if let ExprKind::Path(path) = &obj.kind {
                    if path.segments.len() == 1 {
                        if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                            let var_name = verum_common::Text::from(id.name.as_str());
                            let field_name = verum_common::Text::from(field.name.as_str());
                            self.init_tracker
                                .initialize_field(&var_name, field_name.clone());

                            // AFFINE FIELD REINITIALIZATION: If this field was previously moved,
                            // reassigning it makes it available again, potentially making the
                            // struct "whole" again.
                            // Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — #affine-partial-move
                            self.affine_tracker
                                .reinitialize_field(id.name.as_str(), field.name.as_str());
                        }
                    }
                }
            }

            // Tuple index assignment: x.0 = value (represented as TupleIndex)
            ExprKind::TupleIndex { expr: obj, index } => {
                if let ExprKind::Path(path) = &obj.kind {
                    if path.segments.len() == 1 {
                        if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                            let var_name = verum_common::Text::from(id.name.as_str());
                            let idx = *index as usize;
                            self.init_tracker.initialize_index(&var_name, idx);

                            // AFFINE TUPLE ELEMENT REINITIALIZATION: If this index was previously moved,
                            // reassigning it makes it available again, potentially making the
                            // tuple "whole" again.
                            // Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — #affine-partial-move
                            self.affine_tracker
                                .reinitialize_index(id.name.as_str(), idx);
                        }
                    }
                }
            }

            // Array index assignment: x[i] = value
            ExprKind::Index { expr: obj, index } => {
                if let ExprKind::Path(path) = &obj.kind {
                    if path.segments.len() == 1 {
                        if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                            let var_name = verum_common::Text::from(id.name.as_str());
                            // Try to extract a constant index
                            if let Some(idx) = self.try_extract_const_index(index) {
                                if idx >= 0 {
                                    self.init_tracker.initialize_index(&var_name, idx as usize);
                                }
                            }
                            // Non-constant indices can't be tracked precisely
                        }
                    }
                }
            }

            _ => {
                // Other assignment targets - don't update init state
            }
        }
    }

    /// Track assignment to pattern variables for definite assignment analysis.
    ///

    /// Walks a pattern and marks all bound variables as initialized.
    /// This is used for destructuring assignment: `(a, b) = value`.
    ///

    /// Spec: L0-critical/memory-safety/uninitialized
    fn track_pattern_assignment(
        &mut self,
        pattern: &verum_ast::pattern::Pattern,
        _span: verum_ast::span::Span,
    ) {
        use verum_ast::pattern::PatternKind;

        match &pattern.kind {
            // Identifier pattern - mark the variable as initialized
            PatternKind::Ident {
                name, subpattern, ..
            } => {
                let var_name = verum_common::Text::from(name.name.as_str());
                self.init_tracker.mark_fully_initialized(&var_name);

                // Also track nested subpattern if present
                if let verum_common::Maybe::Some(sub) = subpattern {
                    self.track_pattern_assignment(sub, _span);
                }
            }

            // Wildcard pattern - no variable to track
            PatternKind::Wildcard => {}

            // Rest pattern - no variable to track (collects remaining elements)
            PatternKind::Rest => {}

            // Tuple pattern - recursively track each element
            PatternKind::Tuple(elements) => {
                for elem in elements.iter() {
                    self.track_pattern_assignment(elem, _span);
                }
            }

            // Array pattern - recursively track each element
            PatternKind::Array(elements) => {
                for elem in elements.iter() {
                    self.track_pattern_assignment(elem, _span);
                }
            }

            // Record/struct pattern - recursively track each field
            PatternKind::Record { fields, .. } => {
                for field in fields.iter() {
                    // Field patterns can have explicit patterns or be shorthand
                    if let verum_common::Maybe::Some(pat) = &field.pattern {
                        self.track_pattern_assignment(pat, _span);
                    } else {
                        // Shorthand: `{ x }` means bind x
                        let var_name = verum_common::Text::from(field.name.name.as_str());
                        self.init_tracker.mark_fully_initialized(&var_name);
                    }
                }
            }

            // Variant pattern - track inner data if present
            PatternKind::Variant { data, .. } => {
                if let verum_common::Maybe::Some(variant_data) = data {
                    match variant_data {
                        verum_ast::pattern::VariantPatternData::Tuple(patterns) => {
                            for pat in patterns.iter() {
                                self.track_pattern_assignment(pat, _span);
                            }
                        }
                        verum_ast::pattern::VariantPatternData::Record { fields, .. } => {
                            for field in fields.iter() {
                                if let verum_common::Maybe::Some(pat) = &field.pattern {
                                    self.track_pattern_assignment(pat, _span);
                                } else {
                                    let var_name =
                                        verum_common::Text::from(field.name.name.as_str());
                                    self.init_tracker.mark_fully_initialized(&var_name);
                                }
                            }
                        }
                    }
                }
            }

            // Parenthesized pattern - unwrap and recurse
            PatternKind::Paren(inner) => {
                self.track_pattern_assignment(inner, _span);
            }

            // Or pattern - track all alternatives (they should bind the same variables)
            PatternKind::Or(alternatives) => {
                for alt in alternatives.iter() {
                    self.track_pattern_assignment(alt, _span);
                }
            }

            // And pattern - track all combined patterns
            PatternKind::And(patterns) => {
                for pat in patterns.iter() {
                    self.track_pattern_assignment(pat, _span);
                }
            }

            // Reference pattern - track the inner pattern
            PatternKind::Reference { inner, .. } => {
                self.track_pattern_assignment(inner, _span);
            }

            // Slice pattern - track before, rest, and after patterns
            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                for pat in before.iter() {
                    self.track_pattern_assignment(pat, _span);
                }
                if let verum_common::Maybe::Some(rest_pat) = rest {
                    self.track_pattern_assignment(rest_pat, _span);
                }
                for pat in after.iter() {
                    self.track_pattern_assignment(pat, _span);
                }
            }

            // View pattern - track the inner pattern
            #[allow(deprecated)]
            PatternKind::View { pattern: inner, .. } => {
                self.track_pattern_assignment(inner, _span);
            }

            // TypeTest pattern - mark the binding as initialized
            PatternKind::TypeTest { binding, .. } => {
                let var_name = verum_common::Text::from(binding.name.as_str());
                self.init_tracker.mark_fully_initialized(&var_name);
            }

            // Stream pattern - track head patterns and rest binding
            PatternKind::Stream {
                head_patterns,
                rest,
            } => {
                for pat in head_patterns.iter() {
                    self.track_pattern_assignment(pat, _span);
                }
                if let verum_common::Maybe::Some(rest_ident) = rest {
                    let var_name = verum_common::Text::from(rest_ident.name.as_str());
                    self.init_tracker.mark_fully_initialized(&var_name);
                }
            }

            // Guard pattern - track inner pattern bindings
            // Spec: Rust RFC 3637 - Guard Patterns
            PatternKind::Guard { pattern, .. } => {
                self.track_pattern_assignment(pattern, _span);
            }

            // Cons pattern - track head and tail patterns
            PatternKind::Cons { head, tail } => {
                self.track_pattern_assignment(head, _span);
                self.track_pattern_assignment(tail, _span);
            }

            // Patterns that don't bind variables directly
            PatternKind::Literal(_) | PatternKind::Range { .. } | PatternKind::Active { .. } => {}
        }
    }

    /// Check aliasing constraints for assignment.
    /// An assignment like `data[i] = value` requires exclusive (mutable) access to `data`.
    /// If there are existing immutable borrows, this is an aliasing violation.
    /// Spec: L0-critical/reference_system/access_rules/ref_conflict_error
    fn check_assignment_aliasing(&mut self, left: &Expr, span: Span) -> Result<()> {
        use verum_ast::expr::ExprKind;

        match &left.kind {
            // Variable assignment: x = value
            // Check if there are active borrows of x
            ExprKind::Path(path) => {
                if let Some(verum_ast::ty::PathSegment::Name(id)) = path.segments.first() {
                    let var_name = id.name.as_str();
                    // Check for borrow conflict - assignment requires exclusive access
                    if let Some(err) = self
                        .borrow_tracker
                        .check_borrow_allowed(var_name, true, span)
                    {
                        return Err(err);
                    }
                }
            }

            // Index assignment: data[i] = value
            // The whole collection needs to be mutably borrowed
            ExprKind::Index {
                expr: collection, ..
            } => {
                if let Some(collection_name) = self.extract_base_name(collection) {
                    // Check for borrow conflict - mutation requires exclusive access
                    if let Some(err) =
                        self.borrow_tracker
                            .check_borrow_allowed(&collection_name, true, span)
                    {
                        return Err(err);
                    }
                }
            }

            // Field assignment: container.field = value
            // Check if there are borrows of the field or the whole container
            ExprKind::Field {
                expr: receiver,
                field,
            } => {
                if let Some((base_name, _field_path)) =
                    self.extract_field_path(receiver, field.name.as_str())
                {
                    // Check for borrow conflict - field mutation requires exclusive access
                    if let Some(err) = self
                        .borrow_tracker
                        .check_borrow_allowed(&base_name, true, span)
                    {
                        return Err(err);
                    }
                }
            }

            _ => {
                // Other assignment targets - no aliasing check needed
            }
        }

        Ok(())
    }

    /// Try to extract a constant index from an expression.
    fn try_extract_const_index(&mut self, expr: &Expr) -> Option<i64> {
        use verum_ast::expr::ExprKind;
        use verum_ast::literal::LiteralKind;

        // First try simple literal extraction
        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(int_lit) => {
                    return Some(int_lit.value as i64);
                }
                _ => {}
            },
            // Handle unary negation: -5, -10, etc.
            ExprKind::Unary {
                op: verum_ast::expr::UnOp::Neg,
                expr: inner,
            } => {
                if let ExprKind::Literal(lit) = &inner.kind {
                    if let LiteralKind::Int(int_lit) = &lit.kind {
                        return Some(-(int_lit.value as i64));
                    }
                }
            }
            _ => {}
        }

        // Fall back to const_eval for paths (const variables) and expressions
        // This handles: SIZE, SIZE + 1, SIZE * 2 - 3, OFFSET - 5, etc.
        match self.const_eval.eval(expr) {
            Ok(const_val) => {
                // Convert i128 to i64, safe for array indices
                const_val
                    .as_i128()
                    .map(|n| n as i64)
                    .or_else(|| const_val.as_u128().map(|n| n as i64))
            }
            Err(_) => None,
        }
    }

    /// Get the static size of an array type if known.
    /// Returns None for dynamically-sized arrays, slices, or non-array types.
    fn get_array_size(ty: &Type) -> Option<u64> {
        match ty {
            Type::Array {
                size: Some(size), ..
            } => Some(*size as u64),
            Type::Reference { inner, .. } => Self::get_array_size(inner),
            Type::CheckedReference { inner, .. } => Self::get_array_size(inner),
            Type::UnsafeReference { inner, .. } => Self::get_array_size(inner),
            Type::Ownership { inner, .. } => Self::get_array_size(inner),
            _ => None,
        }
    }

    /// Synthesize type for an expression that's the object of field/index access.
    ///

    /// This is like synth_expr but skips the full initialization check for simple paths,
    /// since field/index access checks the specific field/index instead.
    fn synth_expr_for_field_access(&mut self, expr: &Expr) -> Result<InferResult> {
        use verum_ast::expr::ExprKind;

        // For simple paths, skip the full initialization check - the caller handles field-specific checks
        if let ExprKind::Path(path) = &expr.kind {
            if path.segments.len() == 1 {
                if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                    let name = id.name.as_str();
                    // Just look up the type without initialization checking
                    if let Some(scheme) = self.ctx.env.lookup(name) {
                        let ty = scheme.instantiate();

                        // CRITICAL: Apply unifier to resolve type variables
                        // When we have `wrapper: Wrapper<τ59>` and τ59 was unified with Text,
                        // we need to return `Wrapper<Text>` so field access works correctly.
                        let resolved_ty = self.unifier.apply(&ty);
                        // For GAT/HKT types: if the resolved type contains TypeApp projections,
                        // normalize to reduce them (e.g., C.Item<T> → List<Int> after C and T
                        // have been unified with concrete types).
                        let resolved_ty = if Self::contains_type_app(&resolved_ty) {
                            self.normalize_type(&resolved_ty)
                        } else {
                            resolved_ty
                        };

                        // Check affine usage - detect use after move
                        // Spec: L0-critical/reference_system/value_transfer - Affine type safety
                        // Field access BORROWS the value rather than consuming it.
                        // This allows multiple field accesses on the same affine value:
                        //  assert(handle.id == 1);
                        //  assert(handle.name == "resource");
                        // Only full value transfer (let x = handle) consumes the value.
                        self.affine_tracker.borrow_value(name, expr.span)?;

                        return Ok(InferResult::new(resolved_ty));
                    }
                    // Fall through to full synth_expr for module lookups
                }
            }
        }

        // For other expressions, use normal synth_expr
        self.synth_expr(expr)
    }

    /// Type-check an expression that is the target of an assignment.
    ///

    /// This is like check_expr but skips initialization checking,
    /// since the assignment is what initializes the variable.
    /// Does `ty` have a method `method_name` reachable WITHOUT any
    /// auto-deref? Checks both the inherent methods table (for
    /// `implement T { fn m(...) }` cases) and the protocol-impl set
    /// for `ty` (via `lookup_all_protocol_methods`), plus the
    /// dyn-protocol path when `ty` is a `DynProtocol`.
    ///

    /// Used by the auto-deref cascade in method resolution — the
    /// cascade MUST NOT unwrap `Mutex<T>` to `T` when the user
    /// actually called `mutex.lock()`, so we stop the chain as soon
    /// as `ty` itself owns the method.
    fn type_or_dyn_has_method(&self, ty: &Type, method_name: &Text) -> bool {
        // Peel a single reference layer so `&dyn Tracer` and `&T` are
        // treated as their underlying type for the purposes of method
        // lookup. This keeps the auto-deref cascade halt-condition in
        // sync with the resolver below: if the underlying type owns
        // the method, we must stop walking Deref::Target.
        let inner_ty: &Type = match ty {
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. } => inner.as_ref(),
            _ => ty,
        };
        // Inherent method on the exact type name.
        let type_name: Text = self.type_to_name(inner_ty).to_string().into();
        {
            let inherents = self.inherent_methods.read();
            if let Some(methods) = inherents.get(&type_name) {
                if methods.contains_key(method_name) {
                    return true;
                }
                // Also try the static-method marker form.
                let static_key: Text = format!("$static${}", method_name.as_str()).into();
                if methods.contains_key(&static_key) {
                    return true;
                }
            }
        }
        // Dyn-protocol direct method check.
        if let Type::DynProtocol { bounds, .. } = inner_ty {
            let checker = self.protocol_checker.read();
            for proto_name in bounds.iter() {
                if let Maybe::Some(proto) = checker.get_protocol(proto_name) {
                    if proto.methods.contains_key(method_name) {
                        return true;
                    }
                }
            }
            return false;
        }
        // Protocol-impl lookup for the concrete type.
        let checker = self.protocol_checker.read();
        match checker.lookup_all_protocol_methods(inner_ty, method_name) {
            Ok(candidates) => !candidates.is_empty(),
            Err(_) => false,
        }
    }

    /// Does `ty` directly define a field named `field_name`?
    /// Used by the auto-deref cascade in assignment-target field
    /// resolution — we only need a structural lookup, not full
    /// unification.
    fn type_has_field(&self, ty: &Type, field_name: &Text) -> bool {
        match ty {
            Type::Record(fields) => fields.contains_key(field_name),
            Type::Named { path, .. } => {
                let type_name = self.path_to_string(path);
                let struct_key = format!("__struct_fields_{}", type_name);
                matches!(
                    self.ctx.lookup_type(struct_key.as_str()),
                    Option::Some(Type::Record(fields)) if fields.contains_key(field_name)
                )
            }
            Type::Text => {
                matches!(
                    self.ctx.lookup_type("__struct_fields_Text"),
                    Option::Some(Type::Record(fields)) if fields.contains_key(field_name)
                )
            }
            _ => false,
        }
    }

    fn check_expr_assignment_target(
        &mut self,
        expr: &Expr,
        expected: &Type,
    ) -> Result<InferResult> {
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            // For simple paths (variable assignment), skip init checking
            ExprKind::Path(path) if path.segments.len() == 1 => {
                if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                    let name = id.name.as_str();
                    if let Some(scheme) = self.ctx.env.lookup(name) {
                        let ty = scheme.instantiate();
                        self.unifier.unify(&ty, expected, expr.span)?;
                        return Ok(InferResult::new(ty));
                    }
                }
                // Fall through to normal check
                self.check_expr(expr, expected)
            }

            // For field access (struct.field = value), skip init checking for the field
            ExprKind::Field { expr: obj, field } => {
                // Get the object type without init checking
                let obj_result = self.synth_expr_for_field_access(obj)?;
                let dereferenced_ty = self.unwrap_reference_type(&obj_result.ty);
                let mut normalized_ty = self.normalize_type(dereferenced_ty);

                // Never propagation: any field access on Never produces Never
                if matches!(normalized_ty, Type::Never) {
                    return Ok(InferResult::new(Type::Never));
                }

                // Auto-deref cascade for smart-pointer receivers in
                // assignment-target position. Without this,
                //  let mut g = mutex.lock().unwrap();
                //  g.val = 100;
                // fails with "field 'val' not found in type 'MutexGuard'"
                // even though MutexGuard impls DerefMut<Target = Inner>.
                // We walk the Deref::Target chain (bounded to 8 hops
                // to catch accidental cycles) until we find a type
                // that actually has the field, or until no further
                // Target is defined.
                {
                    let field_name_t: Text = field.name.as_str().into();
                    let mut hops = 0;
                    while hops < 8 && !self.type_has_field(&normalized_ty, &field_name_t) {
                        let next = {
                            let checker = self.protocol_checker.read();
                            checker.try_find_associated_type(
                                &normalized_ty,
                                &verum_common::Text::from("Target"),
                            )
                        };
                        match next {
                            Some(target) => {
                                let unwrapped = self.unwrap_reference_type(&target);
                                normalized_ty = self.normalize_type(unwrapped);
                                hops += 1;
                            }
                            None => break,
                        }
                    }
                }

                // Look up the field type
                match &normalized_ty {
                    Type::Record(fields) => {
                        if let Some(field_ty) =
                            fields.get(&verum_common::Text::from(field.name.as_str()))
                        {
                            self.unifier.unify(field_ty, expected, expr.span)?;
                            Ok(InferResult::new(field_ty.clone()))
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "field '{}' not found in type 'record'",
                                field.name
                            ))))
                        }
                    }
                    Type::Named { path, args } => {
                        let type_name = self.path_to_string(path);
                        let struct_key = format!("__struct_fields_{}", type_name);
                        let field_name = verum_common::Text::from(field.name.as_str());

                        // Look up type parameters for this generic type
                        // so we can substitute T -> Int in Pair<Int>
                        let type_params_key = format!("__type_params_{}", type_name);
                        let type_params: List<verum_common::Text> =
                            match self.ctx.lookup_type(type_params_key.as_str()) {
                                Option::Some(Type::Record(params_map)) => {
                                    params_map.keys().cloned().collect()
                                }
                                _ => List::new(),
                            };

                        // Build substitution map from type parameters to concrete args
                        // e.g., for Pair<Int>: { T -> Int }
                        let mut param_subst = indexmap::IndexMap::new();
                        for (param_name, arg_ty) in type_params.iter().zip(args.iter()) {
                            param_subst.insert(param_name.clone(), arg_ty.clone());
                        }

                        if let Option::Some(Type::Record(fields)) =
                            self.ctx.lookup_type(&struct_key)
                        {
                            if let Some(field_ty) = fields.get(&field_name) {
                                // Apply type parameter substitution to get concrete field type
                                let resolved_ty =
                                    self.substitute_type_params(field_ty, &param_subst);
                                self.unifier.unify(&resolved_ty, expected, expr.span)?;
                                return Ok(InferResult::new(resolved_ty));
                            }
                        }
                        Err(TypeError::Other(verum_common::Text::from(format!(
                            "field '{}' not found in type '{}'",
                            field.name, type_name
                        ))))
                    }
                    // CRITICAL: Allow field access on `Type::Text` when user-defined struct fields exist.
                    // text.vr defines `public type Text is { ptr, len, cap }` and registers
                    // `__struct_fields_Text`. This allows `self.len`, `self.ptr`, etc. inside
                    // `implement Text` blocks to work correctly even though the compiler resolves
                    // `TypeKind::Text` to the primitive `Type::Text`.
                    Type::Text => {
                        let struct_key = "__struct_fields_Text".to_string();
                        if let Some(Type::Record(fields)) = self.ctx.lookup_type(&struct_key) {
                            let field_name_key = verum_common::Text::from(field.name.as_str());
                            if let Some(field_ty) = fields.get(&field_name_key).cloned() {
                                self.unifier.unify(&field_ty, expected, expr.span)?;
                                return Ok(InferResult::new(field_ty));
                            }
                        }
                        Err(TypeError::OtherWithCode {
                            code: verum_common::Text::from("E103"),
                            msg: verum_common::Text::from(format!(
                                "Cannot access field on non-record type: {}",
                                normalized_ty
                            )),
                        })
                    }
                    _ => Err(TypeError::OtherWithCode {
                        code: verum_common::Text::from("E103"),
                        msg: verum_common::Text::from(format!(
                            "Cannot access field on non-record type: {}",
                            normalized_ty
                        )),
                    }),
                }
            }

            // For tuple index (tuple.0 = value), skip init checking for the element
            ExprKind::TupleIndex { expr: tup, index } => {
                let tup_result = self.synth_expr_for_field_access(tup)?;
                match &tup_result.ty {
                    Type::Tuple(types) => {
                        let idx = *index as usize;
                        if idx < types.len() {
                            self.unifier.unify(&types[idx], expected, expr.span)?;
                            Ok(InferResult::new(types[idx].clone()))
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "Tuple index {} out of bounds",
                                index
                            ))))
                        }
                    }
                    // Handle named tuple structs: type Color is (Int, Int, Int)
                    // Also handle newtypes: type UserId is (Int);
                    Type::Named { path, .. } => {
                        let type_name = self.path_to_string(path);
                        let simple_name = Self::path_type_name(path).unwrap_or(&type_name);
                        let idx = *index as usize;

                        // Try tuple struct first (__tuple_fields_)
                        let tuple_fields_key = format!("__tuple_fields_{}", type_name);
                        let tuple_fields_simple_key = format!("__tuple_fields_{}", simple_name);

                        let found_tuple = self
                            .ctx
                            .lookup_type(tuple_fields_key.as_str())
                            .or_else(|| self.ctx.lookup_type(tuple_fields_simple_key.as_str()));

                        if let Option::Some(Type::Tuple(types)) = found_tuple {
                            if idx < types.len() {
                                self.unifier.unify(&types[idx], expected, expr.span)?;
                                Ok(InferResult::new(types[idx].clone()))
                            } else {
                                Err(TypeError::Other(verum_common::Text::from(format!(
                                    "Tuple struct index {} out of bounds (has {} fields)",
                                    index,
                                    types.len()
                                ))))
                            }
                        }
                        // Try newtype (__newtype_inner_) - newtypes support .0 for inner value
                        else {
                            let newtype_inner_key = format!("__newtype_inner_{}", type_name);
                            let newtype_simple_key = format!("__newtype_inner_{}", simple_name);

                            let found_inner = self
                                .ctx
                                .lookup_type(newtype_inner_key.as_str())
                                .or_else(|| self.ctx.lookup_type(newtype_simple_key.as_str()));

                            if let Option::Some(inner_ty) = found_inner {
                                if idx == 0 {
                                    self.unifier.unify(inner_ty, expected, expr.span)?;
                                    Ok(InferResult::new(inner_ty.clone()))
                                } else {
                                    Err(TypeError::Other(verum_common::Text::from(format!(
                                        "Newtype {} only has index 0, not {}",
                                        type_name, index
                                    ))))
                                }
                            } else {
                                Err(TypeError::Other(verum_common::Text::from(format!(
                                    "cannot index type '{}' — only tuple types support .0, .1, etc.",
                                    tup_result.ty
                                ))))
                            }
                        }
                    }
                    // Handle references to named tuple structs: &UserId where UserId is (Int)
                    // Auto-dereference and index the underlying type
                    Type::Reference { inner, .. }
                    | Type::CheckedReference { inner, .. }
                    | Type::UnsafeReference { inner, .. } => {
                        if let Type::Named { path, .. } = inner.as_ref() {
                            let type_name = self.path_to_string(path);
                            let idx = *index as usize;

                            // Try tuple struct first (__tuple_fields_)
                            let tuple_fields_key = format!("__tuple_fields_{}", type_name);
                            if let Option::Some(Type::Tuple(types)) =
                                self.ctx.lookup_type(tuple_fields_key.as_str())
                            {
                                if idx < types.len() {
                                    self.unifier.unify(&types[idx], expected, expr.span)?;
                                    Ok(InferResult::new(types[idx].clone()))
                                } else {
                                    Err(TypeError::Other(verum_common::Text::from(format!(
                                        "Tuple struct index {} out of bounds (has {} fields)",
                                        index,
                                        types.len()
                                    ))))
                                }
                            }
                            // Try newtype (__newtype_inner_) - newtypes support .0 for inner value
                            else {
                                let newtype_inner_key = format!("__newtype_inner_{}", type_name);
                                // Also try with just the last segment of the path (for local types)
                                let simple_name = Self::path_type_name(path).unwrap_or(&type_name);
                                let newtype_simple_key = format!("__newtype_inner_{}", simple_name);

                                let found_inner = self
                                    .ctx
                                    .lookup_type(newtype_inner_key.as_str())
                                    .or_else(|| self.ctx.lookup_type(newtype_simple_key.as_str()));

                                if let Option::Some(inner_ty) = found_inner {
                                    if idx == 0 {
                                        self.unifier.unify(inner_ty, expected, expr.span)?;
                                        Ok(InferResult::new(inner_ty.clone()))
                                    } else {
                                        Err(TypeError::Other(verum_common::Text::from(format!(
                                            "Newtype {} only has index 0, not {}",
                                            type_name, index
                                        ))))
                                    }
                                } else {
                                    Err(TypeError::Other(verum_common::Text::from(format!(
                                        "cannot index type '{}' — only tuple types support .0, .1, etc.",
                                        tup_result.ty
                                    ))))
                                }
                            }
                        } else if let Type::Tuple(types) = inner.as_ref() {
                            // Also handle reference to bare tuple
                            let idx = *index as usize;
                            if idx < types.len() {
                                self.unifier.unify(&types[idx], expected, expr.span)?;
                                Ok(InferResult::new(types[idx].clone()))
                            } else {
                                Err(TypeError::Other(verum_common::Text::from(format!(
                                    "Tuple index {} out of bounds",
                                    index
                                ))))
                            }
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "cannot index type '{}' — only tuple types support .0, .1, etc.",
                                tup_result.ty
                            ))))
                        }
                    }
                    _ => Err(TypeError::Other(verum_common::Text::from(format!(
                        "cannot index type '{}' — only tuple types support .0, .1, etc.",
                        tup_result.ty
                    )))),
                }
            }

            // For array index (arr[i] = value), skip init checking for the element
            ExprKind::Index { expr: arr, index } => {
                let arr_result = self.synth_expr_for_field_access(arr)?;
                // Resolve type variables before index protocol resolution
                let resolved_arr_ty = self.unifier.apply(&arr_result.ty);

                // =========================================================================
                // Protocol-based Index resolution
                // Index operator resolution: "x[i]" desugars to Index/IndexMut protocol method calls
                // =========================================================================
                let resolution_opt = self
                    .protocol_checker
                    .read()
                    .resolve_index_protocol(&resolved_arr_ty);
                match resolution_opt {
                    Some(resolution) => {
                        // Check the index has the appropriate type
                        self.check_expr(index, &resolution.key)?;
                        // Unify output with expected and return
                        self.unifier
                            .unify(&resolution.output, expected, expr.span)?;
                        Ok(InferResult::new(resolution.output))
                    }
                    None => Err(TypeError::Other(verum_common::Text::from(format!(
                        "Cannot index type: {}. Type must implement Index protocol.",
                        arr_result.ty
                    )))),
                }
            }

            // For other expressions, use normal check_expr
            _ => self.check_expr(expr, expected),
        }
    }

    // ==================== Call Graph Building for Negative Context Verification ====================
    // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts - Call Graph Verification

    /// Record a function call for call graph building.
    ///

    /// This is called during type inference when a function call is encountered.
    /// The call site information is stored for later use in transitive negative
    /// context verification.
    ///

    /// # Arguments
    ///

    /// * `callee_name` - Name of the function being called
    /// * `span` - Source location of the call
    fn record_call_site(&mut self, callee_name: impl Into<Text>, span: Span) {
        use crate::context_check::CallSiteInfo;

        let callee_name = callee_name.into();

        // Extract line and column from span
        let (line, column) = crate::source_files::span_to_line_column(span);

        let call_site = CallSiteInfo::new(callee_name.clone(), line, column, span);
        self.current_function_call_sites
            .insert(callee_name, call_site);
    }

    /// Check if calling a function would violate negative context constraints.
    ///

    /// This is called during function call type checking to immediately detect
    /// violations of negative context constraints.
    ///

    /// # Arguments
    ///

    /// * `callee_name` - Name of the function being called
    /// * `span` - Source location of the call
    ///

    /// # Returns
    ///

    /// `Ok(())` if the call is valid, or an error if it violates negative constraints
    fn check_negative_context_violation(&self, callee_name: &str, span: Span) -> Result<()> {
        // Check if caller has any negative context constraints
        if let Maybe::Some(ref caller_contexts) = self.current_function_contexts {
            // Check if any excluded contexts would be violated by calling this function
            self.context_checker
                .check_call_negative_constraints(callee_name, span)?;
        }
        Ok(())
    }

    /// Get the context checker for external access (e.g., for testing)
    pub fn get_context_checker(&self) -> &ContextChecker {
        &self.context_checker
    }

    /// Get mutable access to the context checker
    pub fn get_context_checker_mut(&mut self) -> &mut ContextChecker {
        &mut self.context_checker
    }

    /// Perform transitive negative context verification for all registered functions.
    ///

    /// This should be called after all functions in a module have been type-checked
    /// to verify that no function with negative context constraints transitively
    /// calls a function that uses those contexts.
    ///

    /// # Returns
    ///

    /// `Ok(())` if all functions satisfy their negative context constraints,
    /// or an error with details about the first violation found.
    pub fn verify_all_negative_contexts(&self) -> Result<()> {
        self.context_checker.verify_all_negative_contexts()
    }

    /// Type check a list of items and then verify negative context constraints.
    ///

    /// This is the recommended entry point for checking a module's items.
    /// It performs:
    /// 1. Type checking of all items (functions, types, impls, etc.)
    /// 2. Transitive negative context verification
    ///

    /// # Arguments
    ///

    /// * `items` - The top-level items to check
    ///

    /// # Returns
    ///

    /// `Ok(())` if all items type-check correctly and negative context constraints
    /// are satisfied, or the first error encountered.
    ///

    /// # Example
    ///

    /// ```verum
    /// // This will be caught by transitive verification:
    /// fn uses_db() using [Database] { ... }
    /// fn no_db() using [!Database] { uses_db(); } // Error!
    /// ```
    pub fn check_items_with_negative_context_verification(
        &mut self,
        items: &[verum_ast::Item],
    ) -> Result<()> {
        // Phase -1: Pre-register all modules BEFORE any content processing
        // This enables cross-module imports to work even when modules are declared after
        // the modules that import from them.
        // Module declaration: inline "module name { ... }" or file-based (foo.vr defines module foo) — Module registration order
        for item in items {
            if let verum_ast::ItemKind::Module(module_decl) = &item.kind {
                self.pre_register_module(module_decl, "cog");
            }
        }

        // Phase -0.5: Collect every explicitly-imported name across the
        // file's `mount` declarations BEFORE any of those mounts run.
        //

        // Why: an explicit `mount X.{Bar}` is supposed to be authoritative
        // for the name `Bar` for the rest of the file. The existing guard
        // at `import_item_from_module_impl` (search "explicit_imports")
        // prevents glob re-imports from clobbering an explicit one — but
        // only when the explicit was processed first. When `mount foo.*`
        // appears before the explicit mount in source order, the glob runs
        // first, registers `foo::Bar`, and the later explicit `{Bar}`
        // import races against an existing flat-name binding, producing
        // confusing variant-set diagnostics for users.
        //

        // The fix is order-independent: pre-scan every `Mount` item, walk
        // its tree, and seed `explicit_imports` with every leaf-name that
        // *would* be imported explicitly. Then when a glob runs in Phase 2,
        // the existing guard skips those names — explicit always wins
        // regardless of source order.
        //

        // Spec note: this matches the user-intuitive semantics — explicit
        // imports are authoritative, gloss are background.
        for item in items {
            if let verum_ast::ItemKind::Mount(mount_decl) = &item.kind {
                self.collect_explicit_import_names(&mount_decl.tree);
            }
        }

        // Phase 0: Register all type declarations FIRST
        // This is critical for user-defined types like Maybe<T>, Result<T, E> to be available
        // before type-checking functions that use them.
        // Types must be registered before function signatures because function signatures
        // reference types in their parameters and return types.
        for item in items {
            // Skip @cfg-gated items that don't match the current platform
            if !self.cfg_evaluator.should_include(&item.attributes) {
                continue;
            }
            if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                if let Err(e) = self.register_type_declaration(type_decl) {
                    // Soundness-critical errors (positivity, etc.) MUST
                    // abort the build — masking them with `tracing::debug!`
                    // is precisely the gap that lets `verum build` ship
                    // a Berardi-shaped type as a working binary.
                    // Recoverable errors (forward-ref / cross-module
                    // resolution) keep their original log-and-continue
                    // semantics so genuine forward declarations resolve
                    // on the second pass.
                    if e.is_soundness_critical() {
                        return Err(e);
                    }
                    tracing::debug!(
                        "Initial type registration for '{}' failed (may be resolved later): {}",
                        type_decl.name.name.as_str(),
                        e
                    );
                }
            }
        }

        // Phase 1: Register all function signatures (for forward references)
        for item in items {
            // Skip @cfg-gated items that don't match the current platform
            if !self.cfg_evaluator.should_include(&item.attributes) {
                continue;
            }
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                self.register_function_signature(func)?;
            }
            // Register extern block function signatures (FFI declarations)
            if let verum_ast::ItemKind::ExternBlock(extern_block) = &item.kind {
                for func in &extern_block.functions {
                    // Extern functions have no body - just register their signatures
                    let _ = self.register_function_signature(func);
                }
            }
            // Register FFI boundary function signatures
            if let verum_ast::ItemKind::FFIBoundary(ffi_boundary) = &item.kind {
                let mut boundary_fields = indexmap::IndexMap::new();
                for ffi_func in &ffi_boundary.functions {
                    let mut param_types = verum_common::List::new();
                    for (_name, param_ty) in &ffi_func.signature.params {
                        if let Ok(t) = self.ast_to_type(param_ty) {
                            param_types.push(t);
                        }
                    }
                    let ret_type = self
                        .ast_to_type(&ffi_func.signature.return_type)
                        .unwrap_or(Type::Unit);
                    let fn_type = Type::Function {
                        params: param_types,
                        return_type: Box::new(ret_type),
                        contexts: None,
                        type_params: verum_common::List::new(),
                        properties: None,
                    };
                    self.ctx.env.insert(
                        ffi_func.name.name.as_str(),
                        TypeScheme::mono(fn_type.clone()),
                    );
                    let qualified_name =
                        format!("{}.{}", ffi_boundary.name.name, ffi_func.name.name);
                    self.ctx
                        .env
                        .insert(qualified_name.as_str(), TypeScheme::mono(fn_type.clone()));
                    boundary_fields.insert(ffi_func.name.name.clone(), fn_type);
                }
                // Register the boundary name itself as a record namespace
                let boundary_type = Type::Record(boundary_fields);
                self.ctx.env.insert(
                    ffi_boundary.name.name.as_str(),
                    TypeScheme::mono(boundary_type),
                );
            }
        }

        // Phase 1b: Register active pattern declarations
        // Pattern declarations are compiled as functions, but we also need their
        // return types available for type checking active pattern invocations in match arms.
        // Spec: grammar/verum.ebnf line 1817 - pattern_def
        for item in items {
            if let verum_ast::ItemKind::Pattern(pattern_decl) = &item.kind {
                self.register_pattern_declaration(pattern_decl)?;
            }
        }

        // Phase 1c: Pre-register all const declarations (for forward references)
        // Constants defined after functions in source order should still be visible
        // within function bodies. We register their types here so that name resolution
        // succeeds during Phase 2.
        for item in items {
            if let verum_ast::ItemKind::Const(const_decl) = &item.kind {
                if let Ok(const_ty) = self.ast_to_type(&const_decl.ty) {
                    self.ctx
                        .env
                        .insert(const_decl.name.name.as_str(), TypeScheme::mono(const_ty));
                }
            }
        }

        // Phase 2: Type check all items
        for item in items {
            self.check_item(item)?;
        }

        // Phase 2.5: Solve universe constraints accumulated during
        // type checking (Phase A.2). Any `Type(N)` usage or explicit
        // universe polymorphism constraints are resolved here. Errors
        // are logged and deferred to the DependentVerifier orchestrator
        // which may resolve them with a wider cross-module constraint set.
        // Snapshot the constraints before solving — if the solver
        // fails, the orchestrator gets the actual undecided set.
        let pre_solve_constraints: List<crate::universe_solver::UniverseConstraint> = self
            .ctx
            .universe_ctx()
            .constraints()
            .iter()
            .cloned()
            .collect();

        if let Err(e) = self.ctx.solve_universe_constraints() {
            tracing::debug!("Universe constraint solve produced diagnostics: {}", e);
            self.deferred_verification_goals
                .push(DeferredVerificationGoal::UniverseConstraints {
                    constraints: pre_solve_constraints,
                });
        }

        // Phase 3: Verify transitive negative context constraints
        // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
        self.verify_all_negative_contexts()?;

        // Phase 4: Drain deferred soundness-critical errors. These were
        // stashed by helpers whose Rust signature is `()` — typically
        // cross-module pre-passes that import stdlib types. A
        // positivity violation in any of those declarations would
        // otherwise be silently lost; surface them here so the build
        // aborts before reaching codegen.
        if let Some(e) = self.deferred_soundness_errors.pop() {
            // Surface remaining deferred errors as additional
            // diagnostics so the user sees ALL violations, not only
            // the first.
            let mut tail: Vec<TypeError> = std::mem::take(&mut self.deferred_soundness_errors);
            for extra in tail.drain(..) {
                let diag = extra.to_diagnostic();
                self.diagnostics.push(diag);
            }
            return Err(e);
        }

        Ok(())
    }

    /// Pre-register a module and all nested modules (public interface).
    ///

    /// This is called before any content processing to ensure
    /// all modules are available for cross-module imports.
    ///

    /// Module declaration: inline "module name { ... }" or file-based (foo.vr defines module foo) — Inline Modules
    pub fn pre_register_module_public(
        &mut self,
        module: &verum_ast::decl::ModuleDecl,
        parent_path: &str,
    ) {
        self.pre_register_module(module, parent_path);
    }

    /// Pre-register a module and all nested modules.
    ///

    /// This is called in Phase -1 before any content processing to ensure
    /// all modules are available for cross-module imports.
    ///

    /// Module declaration: inline "module name { ... }" or file-based (foo.vr defines module foo) — Inline Modules
    fn pre_register_module(&mut self, module: &verum_ast::decl::ModuleDecl, parent_path: &str) {
        let module_name = module.name.name.as_str();
        let new_path = if parent_path == "cog" {
            format!("cog.{}", module_name)
        } else {
            format!("{}.{}", parent_path, module_name)
        };
        let new_path_text = verum_common::Text::from(new_path.clone());

        // Register the module with full path
        self.inline_modules.insert(new_path_text, module.clone());

        // Also register with short name relative to parent if at crate root
        if parent_path == "cog" {
            self.inline_modules
                .insert(verum_common::Text::from(module_name), module.clone());
        }

        // Recursively pre-register nested modules
        if let Some(items) = &module.items {
            for item in items.iter() {
                if let verum_ast::ItemKind::Module(nested_module) = &item.kind {
                    self.pre_register_module(nested_module, &new_path);
                }
            }
        }
    }

    /// Build and return the call graph from registered functions.
    ///

    /// This is useful for debugging and visualization of the call relationships.
    pub fn build_call_graph(&self) -> crate::context_check::CallGraph {
        self.context_checker.build_call_graph_from_registry()
    }

    // ==================== Dependent Type Integration ====================
    // Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Dependent Types Extension

    /// Verify a dependent type constraint using SMT solver
    ///

    /// This method is the primary integration point for dependent type verification.
    /// It delegates to RefinementChecker which uses SmtDependentTypeChecker.
    pub fn verify_dependent_type(
        &mut self,
        constraint: &crate::dependent_integration::DependentTypeConstraint,
    ) -> Result<crate::refinement::VerificationResult> {
        match self.refinement.verify_dependent_type(constraint) {
            Ok(result) => Ok(result),
            Err(refinement_err) => Err(TypeError::RefinementFailed {
                predicate: refinement_err.message.clone(),
                span: refinement_err.span,
            }),
        }
    }

    /// Enable dependent type checking
    ///

    /// Historical name: used to auto-construct `SmtDependentTypeChecker` in
    /// `RefinementChecker`. Post-cycle-break, callers must also call
    /// `set_dependent_checker(...)` with a backend boxed from
    /// `verum_smt::dependent_backend::SmtDependentTypeChecker` (or another
    /// impl of `DependentTypeChecker`). Kept as a stub for backward
    /// compatibility.
    pub fn enable_dependent_types(&mut self) {
        self.refinement.enable_dependent_types();
    }

    /// Install a concrete dependent type checker (e.g.
    /// `verum_smt::dependent_backend::SmtDependentTypeChecker`).
    pub fn set_dependent_checker(
        &mut self,
        checker: Box<dyn crate::dependent_integration::DependentTypeChecker>,
    ) {
        self.refinement.set_dependent_checker(checker);
    }

    /// Install a concrete SMT backend (e.g.
    /// `verum_smt::refinement_backend::RefinementZ3Backend`).
    ///

    /// Needed post-cycle-break: the default `RefinementChecker::new` no
    /// longer auto-constructs a `Z3Backend`. Call this once during type
    /// checker setup to restore SMT-backed refinement verification.
    pub fn set_smt_backend(&mut self, backend: Box<dyn crate::refinement::SmtBackend>) {
        self.refinement.set_smt_backend(backend);
    }

    /// Check if dependent type checking is enabled
    pub fn has_dependent_types(&self) -> bool {
        self.refinement.has_dependent_types()
    }

    // ==================== Refinement Evidence API ====================
    // Refinement types enhancement: flow-sensitive refinement propagation, evidence tracking for verified predicates — Refinement Evidence Propagation

    /// Get current path evidence as SMT assumptions
    ///

    /// Returns all learned predicates that are known to be true on the
    /// current execution path. These can be passed to `check_with_evidence`
    /// for flow-sensitive refinement verification.
    ///

    /// # Example
    ///

    /// ```verum
    /// fn process(data: List<Int>) -> Int {
    ///  if data.is_empty() { return 0; }
    ///  // get_evidence_assumptions() returns [!data.is_empty()]
    ///  first(data) // Verification uses evidence to prove safety
    /// }
    /// ```
    pub fn get_evidence_assumptions(&self) -> List<verum_ast::expr::Expr> {
        self.refinement_evidence.to_smt_assumptions()
    }

    /// Check if we have evidence that a variable is non-empty
    ///

    /// Useful for checking List/Array bounds safety without SMT query.
    pub fn has_non_empty_evidence(&mut self, var_name: &verum_common::Text) -> bool {
        self.refinement_evidence.has_non_empty_evidence(var_name)
    }

    /// Check if we have evidence that a variable is Some/Ok
    ///

    /// Useful for checking Maybe/Result unwrap safety without SMT query.
    pub fn has_some_or_ok_evidence(&mut self, var_name: &verum_common::Text) -> bool {
        self.refinement_evidence.has_some_or_ok_evidence(var_name)
    }

    /// Check refinement with current path evidence
    ///

    /// This is a convenience method that combines getting evidence
    /// and calling the refinement checker with that evidence.
    ///

    /// # Arguments
    ///

    /// * `value` - The expression being checked
    /// * `refinement` - The refinement type to check against
    ///

    /// # Returns
    ///

    /// `VerificationResult::Valid` if the value satisfies the refinement
    /// given the current path evidence, or `Invalid`/`Unknown` otherwise.
    pub fn check_refinement_with_evidence(
        &mut self,
        value: &verum_ast::expr::Expr,
        refinement: &crate::refinement::RefinementType,
    ) -> Result<crate::refinement::VerificationResult> {
        let evidence: Vec<_> = self.get_evidence_assumptions().into_iter().collect();
        match self
            .refinement
            .check_with_evidence(value, refinement, &evidence, &self.ctx)
        {
            Ok(result) => Ok(result),
            Err(refinement_err) => Err(TypeError::RefinementFailed {
                predicate: refinement_err.message.clone(),
                span: refinement_err.span,
            }),
        }
    }

    /// When a `Type::Named { path: X }` coerces via the newtype-alias path to
    /// its expansion, and that expansion is a refinement, enforce the
    /// refinement on the value expression. Without this the alias
    /// indirection bypasses the refinement check entirely — struct-field
    /// types like `PageNo = Int where |n|{n >= 1}` would accept `0`.
    ///

    /// Spec: tls-quic.md §4.6 (AckRanges), §9 (V3–V7 invariants).
    pub(crate) fn check_refinement_for_expanded_alias(
        &mut self,
        value: &verum_ast::expr::Expr,
        expanded: &Type,
    ) -> Result<()> {
        // Normalize in case the alias expanded to another Named alias that
        // itself resolves to Refined (rare but legal: `type A is B; type B is
        // Int where ...`).
        let normalized = self.normalize_type(expanded);
        let Type::Refined {
            ref base,
            ref predicate,
        } = normalized
        else {
            return Ok(());
        };
        let refinement_type = crate::refinement::RefinementType {
            base_type: (**base).clone(),
            predicate: predicate.clone(),
            span: value.span,
        };
        match self.check_refinement_with_evidence(value, &refinement_type) {
            Ok(crate::refinement::VerificationResult::Invalid { .. }) => {
                // Mirror the call-site policy at `synth_and_check`:
                // only hard-error when the syntactic evaluator
                // confirms the violation; otherwise defer to gradual
                // verification.
                if let verum_common::Maybe::Some(crate::refinement::VerificationResult::Invalid {
                    ..
                }) = self.refinement.syntactic_check_only(value, predicate)
                {
                    let pred_text = format!("{}", predicate);
                    return Err(TypeError::RefinementFailed {
                        predicate: verum_common::Text::from(pred_text),
                        span: value.span,
                    });
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Get evidence statistics for debugging/optimization
    ///

    /// Returns (conditions_added, conditions_used, cache_hits)
    pub fn evidence_stats(&self) -> (usize, usize, usize) {
        self.refinement_evidence.stats()
    }

    /// Clear all refinement evidence
    ///

    /// Called when entering a new function to reset evidence state.
    pub fn clear_refinement_evidence(&mut self) {
        self.refinement_evidence.clear();
    }

    /// Extract a simple variable name from an expression
    ///

    /// Returns Some(name) if the expression is a simple identifier,
    /// or None for complex expressions like method calls or field access.
    fn extract_simple_var_name(&self, expr: &Expr) -> Maybe<verum_common::Text> {
        match &expr.kind {
            ExprKind::Path(path) => {
                if path.segments.len() == 1 {
                    if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                        return Maybe::Some(ident.name.clone());
                    }
                }
                Maybe::None
            }
            ExprKind::Paren(inner) => self.extract_simple_var_name(inner),
            _ => Maybe::None,
        }
    }

    /// Add pattern evidence to the refinement tracker
    ///

    /// When matching a pattern, we learn constraints about the scrutinee.
    /// For example, matching `Some(x)` tells us the value is Some variant.
    fn add_pattern_evidence(
        &mut self,
        pattern: &verum_ast::pattern::Pattern,
        var_name: verum_common::Text,
        span: Span,
    ) {
        use verum_ast::pattern::PatternKind;

        match &pattern.kind {
            // Variant pattern: e.g., Some(x), None, Ok(v), Err(e)
            PatternKind::Variant { path, .. } => {
                // Extract variant name from path
                if let Some(verum_ast::ty::PathSegment::Name(variant_ident)) = path.segments.last()
                {
                    let variant_name = variant_ident.name.as_str();

                    // Create evidence that the variable matches this variant.
                    // Generate a generic `is_<variant>` method name from the variant tag
                    // instead of hardcoding only Some/None/Ok/Err.
                    // This works for any user-defined sum type with variant-checking methods.
                    let method_name = format!("is_{}", variant_name.to_lowercase());

                    // Check if the scrutinee type actually has this method via protocol
                    // implementations. If not, we still add the evidence as it may be
                    // resolved later or used for refinement narrowing.
                    self.refinement_evidence.add_method_evidence(
                        var_name.clone(),
                        &method_name,
                        false, // Not negated - the check is true
                        span,
                    );
                }
            }

            // Literal pattern: e.g., 0, "hello", true
            PatternKind::Literal(lit) => {
                // Create evidence: var == literal
                let var_expr = Expr::ident(verum_ast::ty::Ident::new(var_name.clone(), span));
                let lit_expr = Expr::literal(lit.clone());
                let eq_expr = Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Eq,
                        left: verum_common::Heap::new(var_expr),
                        right: verum_common::Heap::new(lit_expr),
                    },
                    span,
                );
                self.refinement_evidence
                    .add_evidence_from_condition(&eq_expr, span);
            }

            // Range pattern: e.g., 0..10
            PatternKind::Range {
                start,
                end,
                inclusive,
            } => {
                let var_expr = Expr::ident(verum_ast::ty::Ident::new(var_name.clone(), span));

                // Add: var >= start (if start exists)
                if let Maybe::Some(start_lit) = start {
                    // Convert Literal to Expr
                    let start_expr = Expr::literal((**start_lit).clone());
                    let ge_expr = Expr::new(
                        ExprKind::Binary {
                            op: BinOp::Ge,
                            left: verum_common::Heap::new(var_expr.clone()),
                            right: verum_common::Heap::new(start_expr),
                        },
                        span,
                    );
                    self.refinement_evidence
                        .add_evidence_from_condition(&ge_expr, span);
                }

                // Add: var < end (or var <= end if inclusive)
                if let Maybe::Some(end_lit) = end {
                    let op = if *inclusive { BinOp::Le } else { BinOp::Lt };
                    // Convert Literal to Expr
                    let end_expr = Expr::literal((**end_lit).clone());
                    let bound_expr = Expr::new(
                        ExprKind::Binary {
                            op,
                            left: verum_common::Heap::new(var_expr),
                            right: verum_common::Heap::new(end_expr),
                        },
                        span,
                    );
                    self.refinement_evidence
                        .add_evidence_from_condition(&bound_expr, span);
                }
            }

            // Identifier pattern with subpattern: e.g., x @ Some(_)
            PatternKind::Ident { subpattern, .. } => {
                if let Maybe::Some(sub) = subpattern {
                    self.add_pattern_evidence(sub, var_name, span);
                }
            }

            // Or pattern: e.g., Some(_) | None
            // For now, we don't add evidence for OR patterns as the conditions
            // are not easily combined
            PatternKind::Or(_) => {}

            // Other patterns: wildcard, tuple, record, etc.
            // No specific evidence to add
            _ => {}
        }
    }

    // ==================== Flow-Sensitive Type Narrowing ====================

    /// Narrow variable types in the current scope based on a condition expression.
    ///

    /// When entering an if-then branch with condition `x > 0`, this method
    /// narrows the type of `x` from `Int` to `Int{it > 0}` in the current scope.
    fn narrow_variable_types_from_condition(&mut self, condition: &Expr, negated: bool) {
        if negated {
            let neg = crate::refinement_evidence::PathCondition::negate_expr_static(condition);
            self.narrow_variable_types_impl(&neg);
        } else {
            self.narrow_variable_types_impl(condition);
        }
    }

    fn narrow_variable_types_impl(&mut self, condition: &Expr) {
        match &condition.kind {
            ExprKind::Binary { op, left, right } => match op {
                BinOp::And => {
                    self.narrow_variable_types_impl(left);
                    self.narrow_variable_types_impl(right);
                }
                BinOp::Gt | BinOp::Ge | BinOp::Lt | BinOp::Le | BinOp::Eq | BinOp::Ne => {
                    if let Maybe::Some(var_name) = self.extract_simple_var_name(left) {
                        if let Some(scheme) = self.ctx.env.lookup(&var_name) {
                            let current_ty = scheme.ty.clone();
                            if self.is_refineable_type(&current_ty) {
                                let base_type = self.strip_refinement(&current_ty);
                                let it_ident = verum_ast::ty::Ident::new("it", condition.span);
                                let it_expr = Expr::ident(it_ident);
                                let predicate_expr = Expr::new(
                                    ExprKind::Binary {
                                        op: *op,
                                        left: Box::new(it_expr),
                                        right: Box::new((**right).clone()),
                                    },
                                    condition.span,
                                );
                                let predicate = crate::refinement::RefinementPredicate::inline(
                                    predicate_expr,
                                    condition.span,
                                );
                                let refined_ty = Type::Refined {
                                    base: Box::new(base_type),
                                    predicate,
                                };
                                self.ctx.env.insert_mono(var_name, refined_ty);
                            }
                        }
                    } else if let Maybe::Some(var_name) = self.extract_simple_var_name(right) {
                        if let Some(scheme) = self.ctx.env.lookup(&var_name) {
                            let current_ty = scheme.ty.clone();
                            if self.is_refineable_type(&current_ty) {
                                let base_type = self.strip_refinement(&current_ty);
                                let flipped_op = match op {
                                    BinOp::Gt => BinOp::Lt,
                                    BinOp::Ge => BinOp::Le,
                                    BinOp::Lt => BinOp::Gt,
                                    BinOp::Le => BinOp::Ge,
                                    other => *other,
                                };
                                let it_ident = verum_ast::ty::Ident::new("it", condition.span);
                                let it_expr = Expr::ident(it_ident);
                                let predicate_expr = Expr::new(
                                    ExprKind::Binary {
                                        op: flipped_op,
                                        left: Box::new(it_expr),
                                        right: Box::new((**left).clone()),
                                    },
                                    condition.span,
                                );
                                let predicate = crate::refinement::RefinementPredicate::inline(
                                    predicate_expr,
                                    condition.span,
                                );
                                let refined_ty = Type::Refined {
                                    base: Box::new(base_type),
                                    predicate,
                                };
                                self.ctx.env.insert_mono(var_name, refined_ty);
                            }
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn is_refineable_type(&self, ty: &Type) -> bool {
        match ty {
            Type::Int | Type::Float => true,
            Type::Refined { base, .. } => self.is_refineable_type(base),
            _ => false,
        }
    }

    fn strip_refinement(&self, ty: &Type) -> Type {
        match ty {
            Type::Refined { base, .. } => self.strip_refinement(base),
            other => other.clone(),
        }
    }

    // ==================== Type Variable Bounds Management ====================
    // Generic bounds tracking: type parameters carry protocol constraints (e.g., T: Ord) that are checked at instantiation sites

    /// Register protocol bounds for a type variable
    ///

    /// This method tracks which protocols a type variable is constrained to implement.
    /// Used during generic function type checking to enable method resolution on
    /// bounded type variables (e.g., `fn sort<T: Ord>(list: List<T>)`).
    ///

    /// # Parameters
    /// - `var`: The type variable to register bounds for
    /// - `bounds`: List of protocol bounds (e.g., Ord, Clone)
    pub fn register_type_var_bounds(
        &mut self,
        var: TypeVar,
        bounds: List<crate::protocol::ProtocolBound>,
    ) {
        if !bounds.is_empty() {
            // Validate that all bounds are usable as type constraints (not pure Injectable).
            // A pure Injectable context (declared with `context Name { }`) cannot be used
            // as a type bound — only Constraint and ConstraintAndInjectable protocols can.
            for bound in bounds.iter() {
                if let Some(ident) = bound.protocol.as_ident() {
                    let name = ident.name.as_str();
                    let checker = self.protocol_checker.read();
                    if let verum_common::Maybe::Some(proto) =
                        checker.get_protocol(&verum_common::Text::from(name))
                    {
                        if proto.kind == crate::protocol::ProtocolKind::Injectable {
                            // Pure injectable — emit warning but don't block (may be valid in some contexts)
                            // Full error would require span info not available here
                        }
                    }
                }
            }
            self.type_var_bounds.insert(var, bounds);
        }
    }

    /// Get the protocol bounds for a type variable
    ///

    /// Returns the bounds if the type variable has any registered,
    /// or an empty list if the type variable is unbounded.
    pub fn get_type_var_bounds(&self, var: &TypeVar) -> List<crate::protocol::ProtocolBound> {
        self.type_var_bounds
            .get(var)
            .cloned()
            .unwrap_or_else(List::new)
    }

    /// Check if a type variable has a specific protocol bound
    ///

    /// Returns true if the type variable is constrained to implement the given protocol.
    pub fn type_var_has_bound(&self, var: &TypeVar, protocol_name: &str) -> bool {
        if let Some(bounds) = self.type_var_bounds.get(var) {
            for bound in bounds {
                if let Some(ident) = bound.protocol.as_ident() {
                    if ident.name.as_str() == protocol_name {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Transfer bounds from one type variable to another
    ///

    /// Used during unification when a type variable is substituted.
    /// Ensures that bounds are propagated correctly through substitutions.
    pub fn transfer_type_var_bounds(&mut self, from: &TypeVar, to: &TypeVar) {
        if let Some(bounds) = self.type_var_bounds.get(from).cloned() {
            // Merge with any existing bounds on the target
            if let Some(existing) = self.type_var_bounds.get_mut(to) {
                for bound in bounds {
                    if !existing.iter().any(|b| b.protocol == bound.protocol) {
                        existing.push(bound);
                    }
                }
            } else {
                self.type_var_bounds.insert(*to, bounds);
            }
        }
        // Also transfer direct type bounds
        if let Some(type_bounds) = self.type_var_type_bounds.get(from).cloned() {
            if let Some(existing) = self.type_var_type_bounds.get_mut(to) {
                for bound in type_bounds {
                    if !existing.contains(&bound) {
                        existing.push(bound);
                    }
                }
            } else {
                self.type_var_type_bounds.insert(*to, type_bounds);
            }
        }
    }

    // ==================== Existential Type Verification ====================

    /// Verify that a concrete type satisfies protocol bounds of an existential type variable.
    fn verify_existential_return_bounds(
        &self,
        concrete_ty: &Type,
        existential_var: &TypeVar,
        span: Span,
    ) -> Result<()> {
        use crate::specialization_selection::ProtocolCheckerExt;
        let bounds = self.get_type_var_bounds(existential_var);
        if bounds.is_empty() {
            return Ok(());
        }
        let protocol_checker = self.protocol_checker.read();
        for bound in &bounds {
            if bound.is_negative {
                continue;
            }
            if !protocol_checker.check_protocol_bound(concrete_ty, bound) {
                let protocol_name = bound
                    .protocol
                    .as_ident()
                    .map(|i| i.name.clone())
                    .unwrap_or_else(|| Text::from("?"));
                return Err(TypeError::ExistentialBoundNotSatisfied {
                    witness_type: concrete_ty.to_text(),
                    protocol: protocol_name,
                    span,
                });
            }
        }
        Ok(())
    }

    /// Enter a skolem scope for existential unpacking.
    fn enter_skolem_scope(&mut self) {
        self.skolem_tracker.enter_scope();
    }

    /// Exit a skolem scope, checking that no skolems escape.
    fn exit_skolem_scope(&mut self, result_ty: &Type, span: Span) -> Result<()> {
        let target_level = self.skolem_tracker.current_level().saturating_sub(1);
        if let Some(escaping) = self.skolem_tracker.check_escape(result_ty, target_level) {
            let err = TypeError::ExistentialEscape {
                skolem_name: escaping.name.clone(),
                unpacking_span: escaping.unpacking_span,
                escape_span: span,
            };
            self.skolem_tracker.exit_scope();
            return Err(err);
        }
        self.skolem_tracker.exit_scope();
        Ok(())
    }

    // ==================== Type Variable Direct Type Bounds ====================
    // Generic bounds tracking: type parameters carry protocol constraints (e.g., T: Ord) that are checked at instantiation sites
    //

    // Direct type bounds store actual Type values for type variable constraints.
    // Unlike protocol bounds which reference protocols by path, these are used for:
    // - Function type bounds: F: fn(A) -> B
    // - Equality bounds from generics: T = ConcreteType
    // This enables proper closure type inference when checking against bounded type vars.

    /// Register a direct type bound for a type variable.
    ///

    /// Used for function type bounds like `F: fn() -> T` where the constraint
    /// is an actual type, not a protocol reference.
    pub fn register_type_var_type_bound(&mut self, var: TypeVar, bound: Type) {
        // #[cfg(debug_assertions)]
        // eprintln!("[DEBUG register_type_var_type_bound] Registering var {:?} with bound {}", var, bound.to_text());

        if let Some(existing) = self.type_var_type_bounds.get_mut(&var) {
            if !existing.contains(&bound) {
                existing.push(bound);
            }
        } else {
            self.type_var_type_bounds
                .insert(var, vec![bound].into_iter().collect());
        }
    }

    /// Get direct type bounds for a type variable.
    ///

    /// Returns the list of type constraints (like function types) for the variable,
    /// or an empty list if none are registered.
    pub fn get_type_var_type_bounds(&self, var: &TypeVar) -> List<Type> {
        self.type_var_type_bounds
            .get(var)
            .cloned()
            .unwrap_or_else(List::new)
    }

    /// Try to extract a function type bound from a type variable.
    ///

    /// If the type variable has a function type bound (like `F: fn(A) -> B`),
    /// returns the first such bound. This is essential for closure type inference.
    pub fn get_function_type_bound(&self, var: &TypeVar) -> Maybe<Type> {
        let bounds = self.get_type_var_type_bounds(var);
        // #[cfg(debug_assertions)]
        // eprintln!("[DEBUG get_function_type_bound] Looking up var {:?}, found {} bounds", var, bounds.len());

        for bound in bounds {
            if matches!(bound, Type::Function { .. }) {
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG get_function_type_bound] Found function bound: {}", bound.to_text());
                return Maybe::Some(bound);
            }
        }
        Maybe::None
    }

    // ==================== Deferred Constraint Management ====================
    // Constraint-based type inference: collect type constraints from expressions and solve via unification

    /// Maximum number of deferred constraints before we start draining old ones
    const MAX_DEFERRED_CONSTRAINTS: usize = 50_000;

    /// Add a deferred constraint to be solved later
    ///

    /// Constraints are deferred when they cannot be solved immediately,
    /// typically because type variables are not yet resolved.
    pub fn defer_constraint(&mut self, constraint: DeferredConstraint) {
        // Deduplicate: skip constraints identical to existing ones
        if self.deferred_constraints.contains(&constraint) {
            return;
        }

        // Enforce maximum constraint count to prevent unbounded growth
        if self.deferred_constraints.len() >= Self::MAX_DEFERRED_CONSTRAINTS {
            // Drain the oldest 10% of constraints to make room
            let drain_count = Self::MAX_DEFERRED_CONSTRAINTS / 10;
            eprintln!(
                "Warning: deferred constraints reached limit ({}), draining {} oldest",
                Self::MAX_DEFERRED_CONSTRAINTS,
                drain_count
            );
            drop(self.deferred_constraints.drain(..drain_count));
        }
        self.deferred_constraints.push(constraint);
    }

    /// Process all deferred constraints
    ///

    /// Attempts to solve each deferred constraint with the current substitution.
    /// Returns any constraints that still cannot be solved.
    ///

    /// This is called after each major inference pass to propagate type information.
    pub fn solve_deferred_constraints(&mut self) -> Result<()> {
        // Keep iterating until no more progress is made
        let mut made_progress = true;
        let mut max_iterations = 100; // Prevent infinite loops

        while made_progress && max_iterations > 0 {
            made_progress = false;
            max_iterations -= 1;

            let constraints = std::mem::take(&mut self.deferred_constraints);
            let mut remaining = List::new();

            for constraint in constraints {
                match self.try_solve_constraint(&constraint) {
                    Ok(true) => {
                        // Constraint solved, progress made
                        made_progress = true;
                    }
                    Ok(false) => {
                        // Constraint still cannot be solved, keep it
                        remaining.push(constraint);
                    }
                    Err(e) => {
                        // Constraint failed - this is an actual type error
                        return Err(e);
                    }
                }
            }

            self.deferred_constraints = remaining;
        }

        Ok(())
    }

    /// Attempt to solve a single deferred constraint
    ///

    /// Returns Ok(true) if solved, Ok(false) if still deferred, Err if failed
    fn try_solve_constraint(&mut self, constraint: &DeferredConstraint) -> Result<bool> {
        match constraint {
            DeferredConstraint::Equality { left, right, span } => {
                // For constraint solving, we use the types directly.
                // The unifier handles substitution internally during unification.
                let left_resolved = left.clone();
                let right_resolved = right.clone();

                // Check if both are now ground types (no unresolved variables)
                let left_ground = !self.has_unresolved_vars(&left_resolved);
                let right_ground = !self.has_unresolved_vars(&right_resolved);

                if left_ground && right_ground {
                    // Both resolved - try to unify
                    self.unifier.unify(&left_resolved, &right_resolved, *span)?;
                    Ok(true)
                } else if left_ground || right_ground {
                    // One side resolved - can try to unify
                    self.unifier.unify(&left_resolved, &right_resolved, *span)?;
                    Ok(true)
                } else {
                    // Both still have variables - keep deferred
                    Ok(false)
                }
            }
            DeferredConstraint::ProtocolBound { ty, protocol, span } => {
                let resolved_ty = ty.clone();

                if !self.has_unresolved_vars(&resolved_ty) {
                    // Type is now resolved - check if it implements the protocol
                    if !self
                        .protocol_checker
                        .read()
                        .implements_protocol(&resolved_ty, protocol.as_str())
                    {
                        return Err(TypeError::ProtocolNotImplemented {
                            ty: resolved_ty.to_text(),
                            protocol: protocol.clone(),
                            method: verum_common::Text::from("(pending resolution)"),
                            span: *span,
                        });
                    }
                    Ok(true)
                } else {
                    // Still has unresolved variables
                    Ok(false)
                }
            }
            DeferredConstraint::Subtype { sub, super_, span } => {
                let sub_resolved = sub.clone();
                let super_resolved = super_.clone();

                if !self.has_unresolved_vars(&sub_resolved)
                    && !self.has_unresolved_vars(&super_resolved)
                {
                    // Both resolved - check subtyping
                    if !self.subtyping.is_subtype(&sub_resolved, &super_resolved) {
                        return Err(TypeError::Mismatch {
                            expected: super_resolved.to_text(),
                            actual: sub_resolved.to_text(),
                            span: *span,
                        });
                    }
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            DeferredConstraint::HasMethod {
                receiver_ty,
                method_name,
                expected_signature: _,
                span,
            } => {
                let resolved_ty = receiver_ty.clone();

                if !self.has_unresolved_vars(&resolved_ty) {
                    // Check if the method exists
                    // For now, defer to the protocol checker
                    // This will be validated when the method is actually called
                    let _ = method_name;
                    let _ = span;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            DeferredConstraint::Projection {
                deferred,
                result_var,
                span,
            } => {
                // Try to resolve the projection now that more type information may be available
                use crate::projection::{ProjectionResolver, ProjectionResult};

                // Check if the base type is now resolved
                let base_ty = &deferred.projection.base;
                if self.has_unresolved_vars(base_ty) {
                    // Base type still has unresolved variables - keep deferred
                    return Ok(false);
                }

                // Base type is resolved - try to resolve the projection
                let protocol_checker_guard = self.protocol_checker.read();
                let resolver = ProjectionResolver::new(&protocol_checker_guard, *span);

                match resolver.resolve_projection(&deferred.projection) {
                    Ok(ProjectionResult::Resolved(resolved_ty)) => {
                        // Unify the result variable with the resolved type
                        self.unifier
                            .unify(&Type::Var(*result_var), &resolved_ty, *span)?;
                        Ok(true)
                    }
                    Ok(ProjectionResult::Deferred(_)) => {
                        // Still deferred (shouldn't happen if base is resolved, but handle gracefully)
                        Ok(false)
                    }
                    Err(e) => {
                        // Resolution failed - convert to TypeError
                        Err(e.into())
                    }
                }
            }
            DeferredConstraint::ProjectionBound {
                projection,
                protocol,
                span,
            } => {
                // Try to resolve the projection and check the bound
                use crate::projection::{ProjectionResolver, ProjectionResult};

                // Check if the base type is now resolved
                if self.has_unresolved_vars(&projection.base) {
                    // Base type still has unresolved variables - keep deferred
                    return Ok(false);
                }

                // Try to resolve the projection
                let protocol_checker_guard = self.protocol_checker.read();
                let resolver = ProjectionResolver::new(&protocol_checker_guard, *span);

                match resolver.resolve_projection(projection) {
                    Ok(ProjectionResult::Resolved(resolved_ty)) => {
                        // Check if the resolved type implements the required protocol
                        if !protocol_checker_guard
                            .implements_protocol(&resolved_ty, protocol.as_str())
                        {
                            return Err(TypeError::ProtocolNotSatisfied {
                                ty: resolved_ty.to_text(),
                                protocol: protocol.clone(),
                                span: *span,
                            });
                        }
                        Ok(true)
                    }
                    Ok(ProjectionResult::Deferred(_)) => {
                        // Still deferred
                        Ok(false)
                    }
                    Err(e) => {
                        // Resolution failed - convert to TypeError
                        Err(e.into())
                    }
                }
            }
        }
    }

    /// Recursively constant-fold a refinement predicate expression.
    ///

    /// Used by the dependent-refinement substitution path at call sites
    /// (see the `Type::Function` arm of the Call handler). After
    /// substituting earlier concrete arguments into a later parameter's
    /// predicate, the result may contain pure integer arithmetic that
    /// the syntactic refinement checker cannot decide without reduction
    /// (e.g. `count <= 10 - 5`). This helper reduces such sub-terms to
    /// literals so the resulting predicate is in the shape
    /// `<variable> <op> <literal>`, which the syntactic checker can
    /// trivially evaluate once `<variable>` is also substituted with a
    /// literal at the check site.
    ///

    /// The folding is conservative: only integer arithmetic (+, -, *,
    /// /, %, **) and comparisons (==, !=, <, <=, >, >=) with two
    /// literal operands are reduced. Anything else — variables, calls,
    /// paths, string ops, partially-known expressions — is returned
    /// unchanged. This keeps the transformation sound: a folded
    /// expression is semantically equivalent to the original.
    ///

    /// Division by zero produces the original expression (no panic).
    /// Negation of `Int::MIN` is also preserved as-is to avoid overflow.
    fn const_fold_expr(expr: &Expr) -> Expr {
        use verum_ast::expr::{BinOp, UnOp};
        use verum_ast::literal::{Literal, LiteralKind};

        fn as_int_lit(e: &Expr) -> Option<i128> {
            if let ExprKind::Literal(lit) = &e.kind {
                if let LiteralKind::Int(int_lit) = &lit.kind {
                    return Some(int_lit.value);
                }
            }
            None
        }

        fn as_bool_lit(e: &Expr) -> Option<bool> {
            if let ExprKind::Literal(lit) = &e.kind {
                if let LiteralKind::Bool(b) = &lit.kind {
                    return Some(*b);
                }
            }
            None
        }

        fn make_int(value: i128, span: Span) -> Expr {
            Expr::new(
                ExprKind::Literal(Literal::new(
                    LiteralKind::Int(verum_ast::literal::IntLit::new(value)),
                    span,
                )),
                span,
            )
        }

        fn make_bool(value: bool, span: Span) -> Expr {
            Expr::new(
                ExprKind::Literal(Literal::new(LiteralKind::Bool(value), span)),
                span,
            )
        }

        match &expr.kind {
            // Binary operators: fold operands first, then try to
            // evaluate the op if both are now literals.
            ExprKind::Binary { op, left, right } => {
                let l = TypeChecker::const_fold_expr(left);
                let r = TypeChecker::const_fold_expr(right);

                // Integer arithmetic + comparison
                if let (Some(a), Some(b)) = (as_int_lit(&l), as_int_lit(&r)) {
                    let result_int: Option<i128> = match op {
                        BinOp::Add => a.checked_add(b),
                        BinOp::Sub => a.checked_sub(b),
                        BinOp::Mul => a.checked_mul(b),
                        BinOp::Div if b != 0 => a.checked_div(b),
                        BinOp::Rem if b != 0 => a.checked_rem(b),
                        BinOp::Pow if b >= 0 && b <= u32::MAX as i128 => a.checked_pow(b as u32),
                        _ => None,
                    };
                    if let Some(v) = result_int {
                        return make_int(v, expr.span);
                    }

                    let result_bool: Option<bool> = match op {
                        BinOp::Eq => Some(a == b),
                        BinOp::Ne => Some(a != b),
                        BinOp::Lt => Some(a < b),
                        BinOp::Le => Some(a <= b),
                        BinOp::Gt => Some(a > b),
                        BinOp::Ge => Some(a >= b),
                        _ => None,
                    };
                    if let Some(v) = result_bool {
                        return make_bool(v, expr.span);
                    }
                }

                // Boolean logical ops
                if let (Some(a), Some(b)) = (as_bool_lit(&l), as_bool_lit(&r)) {
                    let result_bool: Option<bool> = match op {
                        BinOp::And => Some(a && b),
                        BinOp::Or => Some(a || b),
                        BinOp::Eq => Some(a == b),
                        BinOp::Ne => Some(a != b),
                        BinOp::Imply => Some(!a || b),
                        BinOp::Iff => Some(a == b),
                        _ => None,
                    };
                    if let Some(v) = result_bool {
                        return make_bool(v, expr.span);
                    }
                }

                // Not foldable — reconstruct with folded children.
                Expr::new(
                    ExprKind::Binary {
                        op: *op,
                        left: Box::new(l),
                        right: Box::new(r),
                    },
                    expr.span,
                )
            }

            // Unary operators on literals.
            ExprKind::Unary { op, expr: inner } => {
                let inner_folded = TypeChecker::const_fold_expr(inner);
                match op {
                    UnOp::Neg => {
                        if let Some(v) = as_int_lit(&inner_folded) {
                            // Preserve i128::MIN as-is; cannot negate without overflow.
                            if let Some(neg) = v.checked_neg() {
                                return make_int(neg, expr.span);
                            }
                        }
                    }
                    UnOp::Not => {
                        if let Some(b) = as_bool_lit(&inner_folded) {
                            return make_bool(!b, expr.span);
                        }
                    }
                    _ => {}
                }
                Expr::new(
                    ExprKind::Unary {
                        op: *op,
                        expr: Box::new(inner_folded),
                    },
                    expr.span,
                )
            }

            // Parenthesised expressions are transparent.
            ExprKind::Paren(inner) => TypeChecker::const_fold_expr(inner),

            // Anything else is returned unchanged. Literals, paths,
            // calls, string ops, etc. are not folded here — the
            // substitution path has already done its job.
            _ => expr.clone(),
        }
    }

    /// Check if a type contains any TypeApp nodes (GAT/HKT applications).
    /// Used to decide whether normalize_type is needed to reduce projections.
    fn contains_type_app(ty: &Type) -> bool {
        match ty {
            Type::TypeApp { .. } => true,
            Type::Tuple(tys) => tys.iter().any(Self::contains_type_app),
            Type::Generic { args, .. } | Type::Named { args, .. } => {
                args.iter().any(Self::contains_type_app)
            }
            Type::Function {
                params,
                return_type,
                ..
            } => params.iter().any(Self::contains_type_app) || Self::contains_type_app(return_type),
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Future { output: inner }
            | Type::GenRef { inner } => Self::contains_type_app(inner),
            Type::Record(fields) => fields.values().any(Self::contains_type_app),
            Type::Variant(variants) => variants.values().any(Self::contains_type_app),
            Type::Array { element, .. } | Type::Slice { element } => {
                Self::contains_type_app(element)
            }
            _ => false,
        }
    }

    /// Check if a type contains associated type projection markers (::Item, ::Output, etc.)
    /// Used to avoid expensive normalize_projection_type calls on types that don't need it.
    fn contains_projection_type(ty: &Type) -> bool {
        match ty {
            Type::Generic { name, args } if name.as_str().starts_with("::") => true,
            Type::Generic { args, .. } | Type::Named { args, .. } => {
                args.iter().any(Self::contains_projection_type)
            }
            Type::Tuple(tys) => tys.iter().any(Self::contains_projection_type),
            Type::Function {
                params,
                return_type,
                ..
            } => {
                params.iter().any(Self::contains_projection_type)
                    || Self::contains_projection_type(return_type)
            }
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => Self::contains_projection_type(inner),
            _ => false,
        }
    }

    /// Check if a type mentions a specific named type anywhere in its structure.
    /// Used to detect recursive type definitions to prevent exponential expansion
    /// during normalization.
    fn type_mentions_name(ty: &Type, name: &str) -> bool {
        match ty {
            Type::Named { path, args } => {
                let type_name = path
                    .segments
                    .last()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                        _ => "",
                    })
                    .unwrap_or("");
                if type_name == name {
                    return true;
                }
                args.iter().any(|a| Self::type_mentions_name(a, name))
            }
            Type::Generic { name: gname, args } => {
                if gname.as_str() == name {
                    return true;
                }
                args.iter().any(|a| Self::type_mentions_name(a, name))
            }
            Type::Variant(variants) => variants.values().any(|v| Self::type_mentions_name(v, name)),
            Type::Tuple(tys) => tys.iter().any(|t| Self::type_mentions_name(t, name)),
            Type::Record(fields) => fields.values().any(|f| Self::type_mentions_name(f, name)),
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Future { output: inner }
            | Type::GenRef { inner } => Self::type_mentions_name(inner, name),
            Type::Function {
                params,
                return_type,
                ..
            } => {
                params.iter().any(|p| Self::type_mentions_name(p, name))
                    || Self::type_mentions_name(return_type, name)
            }
            Type::Array { element, .. } | Type::Slice { element } => {
                Self::type_mentions_name(element, name)
            }
            _ => false,
        }
    }

    /// Collect free type variables from a type, in order of first appearance.
    /// Used for positional substitution in TypeApp with Variant constructors.
    fn collect_free_vars_ordered(ty: &Type) -> Vec<TypeVar> {
        let mut vars = Vec::new();
        let mut seen = std::collections::HashSet::new();
        Self::collect_free_vars_inner(ty, &mut vars, &mut seen);
        vars
    }

    fn collect_free_vars_inner(
        ty: &Type,
        vars: &mut Vec<TypeVar>,
        seen: &mut std::collections::HashSet<TypeVar>,
    ) {
        match ty {
            Type::Var(tv) => {
                if seen.insert(*tv) {
                    vars.push(*tv);
                }
            }
            Type::Generic { args, .. } | Type::Named { args, .. } => {
                for a in args {
                    Self::collect_free_vars_inner(a, vars, seen);
                }
            }
            Type::Tuple(tys) => {
                for t in tys {
                    Self::collect_free_vars_inner(t, vars, seen);
                }
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                for p in params {
                    Self::collect_free_vars_inner(p, vars, seen);
                }
                Self::collect_free_vars_inner(return_type, vars, seen);
            }
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Future { output: inner }
            | Type::GenRef { inner } => Self::collect_free_vars_inner(inner, vars, seen),
            Type::Variant(map) => {
                for v in map.values() {
                    Self::collect_free_vars_inner(v, vars, seen);
                }
            }
            Type::Record(fields) => {
                for v in fields.values() {
                    Self::collect_free_vars_inner(v, vars, seen);
                }
            }
            Type::Array { element, .. } | Type::Slice { element } => {
                Self::collect_free_vars_inner(element, vars, seen);
            }
            Type::TypeApp { constructor, args } => {
                Self::collect_free_vars_inner(constructor, vars, seen);
                for a in args {
                    Self::collect_free_vars_inner(a, vars, seen);
                }
            }
            _ => {}
        }
    }

    /// Check if a type contains unresolved type variables
    fn has_unresolved_vars(&self, ty: &Type) -> bool {
        match ty {
            Type::Var(_) => true,
            Type::Function {
                params,
                return_type,
                ..
            } => {
                params.iter().any(|p| self.has_unresolved_vars(p))
                    || self.has_unresolved_vars(return_type)
            }
            Type::Generic { args, .. } | Type::Named { args, .. } => {
                args.iter().any(|a| self.has_unresolved_vars(a))
            }
            Type::Tuple(ts) => ts.iter().any(|t| self.has_unresolved_vars(t)),
            Type::Array { element, .. } => self.has_unresolved_vars(element),
            Type::Slice { element } => self.has_unresolved_vars(element),
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. }
            | Type::Pointer { inner, .. }
            | Type::VolatilePointer { inner, .. } => self.has_unresolved_vars(inner),
            Type::Record(fields) => fields.values().any(|t| self.has_unresolved_vars(t)),
            Type::Variant(variants) => variants.values().any(|t| self.has_unresolved_vars(t)),
            Type::Refined { base, .. } => self.has_unresolved_vars(base),
            Type::Future { output } => self.has_unresolved_vars(output),
            Type::Generator {
                yield_ty,
                return_ty,
            } => self.has_unresolved_vars(yield_ty) || self.has_unresolved_vars(return_ty),
            Type::Pi {
                param_type,
                return_type,
                ..
            } => self.has_unresolved_vars(param_type) || self.has_unresolved_vars(return_type),
            Type::Sigma {
                fst_type, snd_type, ..
            } => self.has_unresolved_vars(fst_type) || self.has_unresolved_vars(snd_type),
            Type::Forall { body, .. } | Type::Exists { body, .. } => self.has_unresolved_vars(body),
            Type::Quantified { inner, .. } => self.has_unresolved_vars(inner),
            // Primitive types have no type variables
            Type::Unit
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Char
            | Type::Text
            | Type::Never
            | Type::Universe { .. }
            | Type::Prop
            | Type::Lifetime { .. }
            | Type::TypeConstructor { .. }
            | Type::Placeholder { .. } => false,
            // For complex types, recursively check
            Type::Meta { ty, .. } => self.has_unresolved_vars(ty),
            Type::Eq { ty, .. } => self.has_unresolved_vars(ty),
            // Path type: Path<A>(a, b) lives in the same universe as A; check space for vars
            Type::PathType { space, .. } => self.has_unresolved_vars(space),
            // Partial element type: Partial<A>(φ) — check element_type for vars
            Type::Partial { element_type, .. } => self.has_unresolved_vars(element_type),
            // Interval is a primitive built-in type with no type parameters
            Type::Interval => false,
            Type::Inductive {
                params, indices, ..
            } => {
                params.iter().any(|(_, t)| self.has_unresolved_vars(t))
                    || indices.iter().any(|(_, t)| self.has_unresolved_vars(t))
            }
            Type::Coinductive { params, .. } => {
                params.iter().any(|(_, t)| self.has_unresolved_vars(t))
            }
            Type::HigherInductive { params, .. } => {
                params.iter().any(|(_, t)| self.has_unresolved_vars(t))
            }
            Type::Tensor { element, .. } => self.has_unresolved_vars(element),
            Type::GenRef { inner } => self.has_unresolved_vars(inner),
            Type::TypeApp { constructor, args } => {
                self.has_unresolved_vars(constructor)
                    || args.iter().any(|a| self.has_unresolved_vars(a))
            }
            // ExtensibleRecord - check fields and row variable
            Type::ExtensibleRecord { fields, row_var } => {
                fields.values().any(|f| self.has_unresolved_vars(f)) || row_var.is_some() // Row variable means potentially unresolved
            }
            // CapabilityRestricted - check base type
            Type::CapabilityRestricted { base, .. } => self.has_unresolved_vars(base),

            // Unknown type - no type variables (it's a concrete top type)
            Type::Unknown => false,

            // DynProtocol - check bindings for unresolved vars
            Type::DynProtocol { bindings, .. } => {
                bindings.values().any(|t| self.has_unresolved_vars(t))
            }
        }
    }

    /// Verify a dependent type constraint with proper error propagation
    ///

    /// Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Dependent Types Verification
    ///

    /// This method verifies the constraint and returns an error if verification fails
    /// and the constraint is proven invalid. Unknown results are treated as tentatively
    /// valid (gradual verification) - they will be checked at runtime.
    fn verify_dependent_type_constraint(
        &mut self,
        constraint: &crate::dependent_integration::DependentTypeConstraint,
        span: Span,
    ) -> Result<()> {
        match self.verify_dependent_type(constraint) {
            Ok(crate::refinement::VerificationResult::Valid) => {
                // Constraint verified - no runtime check needed
                Ok(())
            }
            Ok(crate::refinement::VerificationResult::Invalid { counterexample }) => {
                // Constraint proven invalid - generate error with counterexample
                let mut message =
                    verum_common::Text::from("Dependent type constraint cannot be satisfied");

                if let Maybe::Some(ref ce) = counterexample {
                    message = format!(
                        "Dependent type constraint cannot be satisfied: {} = {}{}",
                        ce.var_name,
                        ce.value,
                        match &ce.explanation {
                            Maybe::Some(expl) => format!(" ({})", expl),
                            Maybe::None => String::new(),
                        }
                    )
                    .into();
                }

                Err(TypeError::RefinementFailed {
                    predicate: message,
                    span,
                })
            }
            Ok(crate::refinement::VerificationResult::Unknown { reason: _ }) => {
                // Unknown result - gradual verification, defer to runtime
                // This allows programs to compile even when SMT cannot prove the constraint
                Ok(())
            }
            Err(e) => {
                // Verification error - treat as unknown (gradual verification)
                // Log the error but don't fail compilation
                let _ = e; // Suppress unused warning
                Ok(())
            }
        }
    }

    /// Convert a generic type argument to an expression
    ///

    /// Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Type-level expressions
    ///

    /// In dependent types, type arguments may contain expression-level values.
    /// This function extracts the expression from a generic argument.
    ///

    /// # Conversion Rules
    /// - GenericArg::Const(expr) - Direct expression, used as-is
    /// - GenericArg::Type(ty) - Try to extract value from type path or compute from type
    /// - GenericArg::Lifetime(lt) - Convert to path expression referencing the lifetime
    /// - GenericArg::Binding(binding) - Recursively convert the associated type
    /// - None - Generate fresh type variable expression for inference
    fn type_arg_to_expr(
        &self,
        arg: Option<&verum_ast::ty::GenericArg>,
        span: Span,
    ) -> verum_ast::expr::Expr {
        use verum_ast::expr::{Expr, ExprKind};
        use verum_ast::literal::{IntLit, Literal, LiteralKind};
        use verum_ast::ty::{GenericArg, PathSegment, TypeKind};

        match arg {
            Some(GenericArg::Const(expr)) => {
                // Const expression - use directly
                expr.clone()
            }
            Some(GenericArg::Type(ty)) => {
                // Type argument - try to extract value from various type forms
                match &ty.kind {
                    TypeKind::Path(path) => {
                        if let Some(PathSegment::Name(ident)) = path.segments.last() {
                            // Check if it's a number literal in type position
                            if let Ok(n) = ident.name.as_str().parse::<i128>() {
                                return Expr::new(
                                    ExprKind::Literal(Literal {
                                        kind: LiteralKind::Int(IntLit::new(n)),
                                        span,
                                    }),
                                    span,
                                );
                            }
                            // Otherwise treat as a variable reference (meta parameter)
                            return Expr::new(ExprKind::Path(path.clone()), span);
                        }
                        // Multi-segment path - return full path as expression
                        Expr::new(ExprKind::Path(path.clone()), span)
                    }
                    TypeKind::Tuple(elements) => {
                        // Convert tuple type to tuple expression with recursive conversion
                        let tuple_exprs: List<Expr> = elements
                            .iter()
                            .map(|elem_ty| {
                                self.type_arg_to_expr(
                                    Some(&GenericArg::Type(elem_ty.clone())),
                                    elem_ty.span,
                                )
                            })
                            .collect();
                        Expr::new(ExprKind::Tuple(tuple_exprs), span)
                    }
                    TypeKind::Array { element: _, size } => {
                        // For array types, extract the size expression if available
                        if let Some(size_expr) = size {
                            (**size_expr).clone()
                        } else {
                            // No explicit size - create fresh type variable for inference
                            self.create_type_variable_expr(span)
                        }
                    }
                    _ => {
                        // For complex types that cannot be converted to expressions,
                        // create a fresh type variable for inference
                        self.create_type_variable_expr(span)
                    }
                }
            }
            Some(GenericArg::Lifetime(lt)) => {
                // Lifetime argument - convert to path expression
                Expr::new(
                    ExprKind::Path(verum_ast::ty::Path {
                        segments: smallvec::smallvec![PathSegment::Name(
                            verum_ast::ty::Ident::new(lt.name.clone(), span)
                        )],
                        span,
                    }),
                    span,
                )
            }
            Some(GenericArg::Binding(binding)) => {
                // Type binding - recursively extract from the associated type
                self.type_arg_to_expr(Some(&GenericArg::Type(binding.ty.clone())), span)
            }
            None => {
                // Missing argument - create fresh type variable for inference
                // The type checker will later unify this with the actual type
                self.create_type_variable_expr(span)
            }
        }
    }

    /// Create a fresh type variable expression for inference.
    ///

    /// This creates a unique identifier that the type checker can use to infer
    /// the actual value during unification. The name follows the pattern `_tv{N}`
    /// Evaluate a `GenericArg::Const` expression to a concrete compile-time value
    /// and wrap it in a `Type::Meta` that carries the value.
    ///

    /// Returns `None` when the expression is not a compile-time constant (e.g. it
    /// references another meta parameter whose value is only known after generic
    /// instantiation); the caller should fall back to type-only representation
    /// for such arguments.
    ///

    /// This is stdlib-agnostic: the base type is derived from the `ConstValue`
    /// kind through its built-in primitive mapping, never by inspecting type
    /// names or hardcoding stdlib knowledge.
    fn eval_const_arg(&self, expr: &verum_ast::expr::Expr) -> Option<Type> {
        let mut evaluator = crate::const_eval::ConstEvaluator::new();
        let value = evaluator.eval(expr).ok()?;
        let base_ty = match &value {
            verum_common::ConstValue::Bool(_) => Type::Bool,
            verum_common::ConstValue::Int(_) | verum_common::ConstValue::UInt(_) => Type::Int,
            verum_common::ConstValue::Float(_) => Type::Float,
            verum_common::ConstValue::Char(_) => Type::Char,
            verum_common::ConstValue::Text(_) => Type::Text,
            // Non-scalar compile-time values (aggregates, optionals, byte literals,
            // unit) aren't yet carried through type-level unification; defer to
            // the caller's fallback so their existing behavior is preserved.
            _ => return None,
        };
        Some(Type::meta_value(value, base_ty))
    }

    /// where N is a unique counter.
    fn create_type_variable_expr(&self, span: Span) -> verum_ast::expr::Expr {
        use std::sync::atomic::{AtomicU64, Ordering};
        use verum_ast::expr::{Expr, ExprKind};
        use verum_ast::ty::{Ident, Path, PathSegment};

        // Use atomic counter for unique type variable names
        static TYPE_VAR_COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = TYPE_VAR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("_tv{}", id);

        Expr::new(
            ExprKind::Path(Path {
                segments: smallvec::smallvec![PathSegment::Name(Ident::new(name.as_str(), span))],
                span,
            }),
            span,
        )
    }

    /// Create a symbolic bounded integer expression for Fin type well-formedness checking.
    ///

    /// Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — .3 - Fin types
    ///

    /// This creates a unique symbolic variable that represents an arbitrary value
    /// in the range [0, n) for a Fin<n> type. The SMT solver uses this to verify
    /// that n is a valid bound (n > 0) during type well-formedness checking.
    ///

    /// The symbolic value is distinct from concrete values:
    /// - Symbolic: Used for type-level well-formedness verification
    /// - Concrete: Used when actual values are assigned to Fin<n> variables
    fn create_symbolic_fin_value(&self, span: Span) -> verum_ast::expr::Expr {
        use std::sync::atomic::{AtomicU64, Ordering};
        use verum_ast::expr::{Expr, ExprKind};
        use verum_ast::ty::{Ident, Path, PathSegment};

        // Use atomic counter for unique symbolic variable names
        static FIN_SYM_COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = FIN_SYM_COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("_fin_sym{}", id);

        Expr::new(
            ExprKind::Path(Path {
                segments: smallvec::smallvec![PathSegment::Name(Ident::new(name.as_str(), span))],
                span,
            }),
            span,
        )
    }

    /// Register ONLY primitive types - used for stdlib bootstrap mode.
    /// In stdlib bootstrap mode, all other types (List, Map, Maybe, etc.)
    /// come from parsing stdlib .vr files, not from hardcoded registration.
    ///

    /// Stdlib bootstrap: dependency-ordered compilation of core .vr modules, type metadata extracted from parsed stdlib files
    ///

    /// Registered types:
    /// - Core: Int, Float, Bool, Text, Char, Unit, Never, Byte
    /// - Sized integers: i8-i128, u8-u128, isize, usize
    /// - Sized floats: f32, f64
    ///

    /// NOT registered (come from stdlib):
    /// - Collections: List<T>, Map<K,V>, Set<T>
    /// - Smart pointers: Heap<T>, Shared<T>, Weak<T>
    /// - Optional types: Maybe<T>, Result<T,E>
    /// - Protocols: Eq, Ord, Clone, Hash, Iterator, etc.
    /// - Domain types: Database, HttpClient, Config, etc.
    pub fn register_primitives(&mut self) {
        // ============================================================
        // PRIMITIVE TYPES
        // These are language built-ins, not stdlib definitions
        // Core type system: primitive types (Bool, Int, Float, Text, Unit), compound types (Array, Tuple, Record, Function)
        // ============================================================
        self.ctx
            .define_type(verum_common::Text::from(WKT::Int.as_str()), Type::Int);
        self.ctx
            .define_type(verum_common::Text::from(WKT::Float.as_str()), Type::Float);
        self.ctx
            .define_type(verum_common::Text::from(WKT::Bool.as_str()), Type::Bool);
        self.ctx
            .define_type(verum_common::Text::from(WKT::Text.as_str()), Type::Text);
        self.ctx
            .define_type(verum_common::Text::from(WKT::Char.as_str()), Type::Char);
        self.ctx
            .define_type(verum_common::Text::from("Unit"), Type::Unit);
        self.ctx
            .define_type(verum_common::Text::from("Never"), Type::Never);
        // Lowercase aliases for primitive types (common in tests)
        self.ctx
            .define_type(verum_common::Text::from("char"), Type::Char);
        self.ctx
            .define_type(verum_common::Text::from("bool"), Type::Bool);
        // Opaque type: abstract type with no known structure (used in verification)
        self.ctx
            .define_type(verum_common::Text::from("opaque"), Type::Unknown);

        // ============================================================
        // SIZED PRIMITIVE TYPES (Semantic Names - Primary)
        // Core type system: primitive types (Bool, Int, Float, Text, Unit), compound types (Array, Tuple, Record, Function)
        // Following Verum's Semantic Honesty principle
        //

        // CRITICAL: Sized integer types are registered as Type::Named, NOT Type::Int!
        // This ensures that their inherent methods (checked_add, saturating_add, etc.)
        // are correctly looked up. If we registered them as Type::Int, method lookup
        // would find Int's methods instead of the type-specific methods.
        // ============================================================
        // Signed integers (semantic names) - MUST be distinct Named types for correct method lookup
        // This ensures Int32.to_le_bytes() returns [Byte; 4] not [Byte; 8]
        self.ctx.define_type(
            verum_common::Text::from("Int8"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "Int8",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("Int16"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "Int16",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("Int32"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "Int32",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("Int64"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "Int64",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("Int128"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "Int128",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("ISize"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "ISize",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("IntSize"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "IntSize",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        // Unsigned integers (semantic names) - MUST be distinct Named types for correct method lookup
        // This ensures UInt64.checked_add uses unsigned overflow detection
        self.ctx.define_type(
            verum_common::Text::from("UInt8"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UInt8",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("UInt16"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UInt16",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("UInt32"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UInt32",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("UInt64"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UInt64",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("UInt128"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UInt128",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("USize"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "USize",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("UIntSize"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UIntSize",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        // Floating point (semantic names) - MUST be distinct Named types for correct method lookup
        // This ensures Float32.to_bits() returns UInt32 not Int, Float64.to_bits() returns UInt64
        self.ctx.define_type(
            verum_common::Text::from("Float32"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "Float32",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("Float64"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "Float64",
                    Span::default(),
                )),
                args: List::new(),
            },
        );

        // ============================================================
        // COMPATIBILITY ALIASES (FFI - Rust/C style names)
        // These map to their semantic equivalents
        // ============================================================
        // Signed integers (compat names -> semantic Named types).
        // Map to the stdlib-defined `Int8` / `Int16` / … so that associated
        // constants (`Int64.MAX`, `Int8.MIN`, …) are reachable via the
        // shorthand `i8.MAX` form used by L2 tests. Previously all aliased
        // to `Type::Int`, which erased the per-width identity and made
        // `i64.MAX` resolve to the bare `Int` type (no MAX constant) at
        // typecheck, then crash with NullPointer at VBC runtime.
        for (compat, canonical) in [
            ("i8", "Int8"),
            ("i16", "Int16"),
            ("i32", "Int32"),
            ("i64", "Int64"),
            ("i128", "Int128"),
            ("isize", "ISize"),
        ] {
            self.ctx.define_type(
                verum_common::Text::from(compat),
                Type::Named {
                    path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                        canonical,
                        Span::default(),
                    )),
                    args: List::new(),
                },
            );
        }
        // Unsigned integers (compat names -> semantic Named types)
        // These map to the same Named types as their semantic equivalents (UInt8, etc.)
        self.ctx.define_type(
            verum_common::Text::from("u8"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UInt8",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("u16"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UInt16",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("u32"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UInt32",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("u64"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UInt64",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("u128"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UInt128",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("usize"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UIntSize",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        // Floating point (compat names -> semantic)
        self.ctx
            .define_type(verum_common::Text::from("f32"), Type::Float);
        self.ctx
            .define_type(verum_common::Text::from("f64"), Type::Float);

        // ============================================================
        // TYPE ALIASES
        // Common aliases used throughout stdlib
        // ============================================================
        // Byte is a distinct type (unsigned 8-bit integer) with its own methods
        // Using Named type so that Byte-specific methods are found
        self.ctx.define_type(
            verum_common::Text::from("Byte"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "Byte",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        // str is an alias for Text (for compatibility)
        self.ctx
            .define_type(verum_common::Text::from("str"), Type::Text);

        // Short aliases for sized types (used throughout stdlib as canonical names)
        // These map to themselves as Named types (U64, U32, etc.)
        // The VBC/stdlib ecosystem uses these short names, not the long forms
        for name in [
            "U8", "U16", "U32", "U64", "U128", "I8", "I16", "I32", "I64", "I128", "F32", "F64",
        ] {
            self.ctx.define_type(
                verum_common::Text::from(name),
                Type::Named {
                    path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                        name,
                        Span::default(),
                    )),
                    args: List::new(),
                },
            );
        }

        // ============================================================
        // C FFI TYPE ALIASES
        // Platform-dependent C type equivalents for FFI interop
        // ============================================================
        self.ctx.define_type(
            verum_common::Text::from("c_char"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "Int8",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("c_uchar"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UInt8",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("c_short"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "Int16",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("c_ushort"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UInt16",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx
            .define_type(verum_common::Text::from("c_int"), Type::Int);
        self.ctx.define_type(
            verum_common::Text::from("c_uint"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UInt32",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx
            .define_type(verum_common::Text::from("c_long"), Type::Int);
        self.ctx.define_type(
            verum_common::Text::from("c_ulong"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UInt64",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("c_longlong"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "Int64",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx
            .define_type(verum_common::Text::from("c_float"), Type::Float);
        self.ctx
            .define_type(verum_common::Text::from("c_double"), Type::Float);
        self.ctx
            .define_type(verum_common::Text::from("c_void"), Type::Unit);
        self.ctx.define_type(
            verum_common::Text::from("c_size_t"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "UIntSize",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("c_ssize_t"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "IntSize",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        // CString and CStr - opaque types for C string interop
        self.ctx.define_type(
            verum_common::Text::from("CString"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "CString",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        self.ctx.define_type(
            verum_common::Text::from("CStr"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "CStr",
                    Span::default(),
                )),
                args: List::new(),
            },
        );
        // Nat - natural numbers (used in dependent type contexts, alias for Int)
        self.ctx
            .define_type(verum_common::Text::from("Nat"), Type::Int);

        // Meta-types for staged metaprogramming
        for meta_type in [
            "@Expr", "@Ident", "@Type", "@Pattern", "@Stmt", "@Item", "@Block",
        ] {
            self.ctx.define_type(
                verum_common::Text::from(meta_type),
                Type::Named {
                    path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                        meta_type,
                        Span::default(),
                    )),
                    args: List::new(),
                },
            );
        }

        // Intrinsic types used in dependent types
        self.ctx.define_type(
            verum_common::Text::from("intrinsic"),
            Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "intrinsic",
                    Span::default(),
                )),
                args: List::new(),
            },
        );

        // ============================================================
        // COMPILER INTRINSICS
        // Spec: These are compiler magic, not stdlib
        // ============================================================
        // Type introspection - registered as generic functions fn<T>() -> Int
        // Note: Also registered in register_meta_builtins with regular arg support.
        // The meta_builtins version overrides this one.
        {
            let tv = TypeVar::fresh();
            let fn_ty = Type::function(List::from_iter([Type::Var(tv)]), Type::Int);
            self.ctx.env.insert(
                verum_common::Text::from("size_of"),
                TypeScheme::poly(List::from_iter([tv]), fn_ty),
            );
        }
        {
            let tv = TypeVar::fresh();
            let fn_ty = Type::function(List::from_iter([Type::Var(tv)]), Type::Int);
            self.ctx.env.insert(
                verum_common::Text::from("align_of"),
                TypeScheme::poly(List::from_iter([tv]), fn_ty),
            );
        }

        // Panic (never returns)
        self.ctx.env.insert(
            verum_common::Text::from("panic"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Text]), Type::Never)),
        );

        // Unreachable (never returns)
        self.ctx.env.insert(
            verum_common::Text::from("unreachable"),
            TypeScheme::mono(Type::function(List::new(), Type::Never)),
        );

        // verum_panic is an alias for panic (used internally in stdlib)
        self.ctx.env.insert(
            verum_common::Text::from("verum_panic"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Text]), Type::Never)),
        );

        // Args count intrinsic (returns number of program arguments)
        self.ctx.env.insert(
            verum_common::Text::from("__verum_args_count"),
            TypeScheme::mono(Type::function(List::new(), Type::Int)),
        );

        // NOTE: __verum_args_get is registered from extern block in core/env.vr
        // Actual FFI signature: (index: Int, buf: &unsafe Byte, buf_len: Int) -> Int
        // We don't register it here to avoid conflicts with extern block registration

        // Assert intrinsic
        self.ctx.env.insert(
            verum_common::Text::from("assert"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Bool]), Type::Unit)),
        );

        // Print and println intrinsics are registered as generic fn<T>(T) -> Unit
        // in register_builtins() below. The Text-only versions from stdlib should NOT
        // override those (protected in register_function_signature).

        // ============================================================
        // FFI INTRINSICS
        // Runtime functions called by stdlib implementations
        // ============================================================

        // Memory intrinsics
        // null_ptr<T>() -> &unsafe T
        let t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("null_ptr"),
            TypeScheme::poly(
                List::from_iter([t]),
                Type::function(
                    List::new(), // No arguments
                    Type::UnsafeReference {
                        inner: Box::new(Type::Var(t)),
                        mutable: false,
                    },
                ),
            ),
        );

        {
            let drop_t = TypeVar::fresh();
            let drop_params = List::from_iter([Type::Var(drop_t)]);
            let drop_ty = Type::function(drop_params, Type::unit());
            self.ctx.env.insert(
                verum_common::Text::from("drop"),
                TypeScheme::poly(List::from_iter([drop_t]), drop_ty),
            );
        }

        // Time intrinsics
        self.ctx.env.insert(
            verum_common::Text::from("verum_time_system_now"),
            TypeScheme::mono(Type::function(List::new(), Type::Int)),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_time_now_ns"),
            TypeScheme::mono(Type::function(List::new(), Type::Int)),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_sleep_ns"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Int]), Type::Unit)),
        );

        // File I/O intrinsics
        self.ctx.env.insert(
            verum_common::Text::from("verum_file_open"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Text, Type::Int]),
                Type::Int, // Returns file descriptor or error code
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_file_read"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Int, Type::Int]),
                Type::Int,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_file_write"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Int, Type::Int]),
                Type::Int,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_file_close"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Int]), Type::Unit)),
        );

        // High-level file I/O functions (path-based)
        // file_write(path: Text, content: Text) -> Int
        self.ctx.env.insert(
            verum_common::Text::from("file_write"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Text, Type::Text]),
                Type::Int,
            )),
        );
        // file_read(path: Text) -> Text
        self.ctx.env.insert(
            verum_common::Text::from("file_read"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Text]), Type::Text)),
        );
        // file_append(path: Text, content: Text) -> Int
        self.ctx.env.insert(
            verum_common::Text::from("file_append"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Text, Type::Text]),
                Type::Int,
            )),
        );
        // file_delete(path: Text) -> Int
        self.ctx.env.insert(
            verum_common::Text::from("file_delete"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Text]), Type::Int)),
        );
        // file_exists(path: Text) -> Int
        self.ctx.env.insert(
            verum_common::Text::from("file_exists"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Text]), Type::Int)),
        );

        // Stdio intrinsics
        self.ctx.env.insert(
            verum_common::Text::from("stdlibin_read_line"),
            TypeScheme::mono(Type::function(List::new(), Type::Text)),
        );

        self.ctx.env.insert(
            verum_common::Text::from("stdlibout_write"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Text]), Type::Unit)),
        );

        self.ctx.env.insert(
            verum_common::Text::from("stdliberr_write"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Text]), Type::Unit)),
        );

        // Atomic/sync intrinsics
        self.ctx.env.insert(
            verum_common::Text::from("verum_fence"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Int]), // Ordering as int
                Type::Unit,
            )),
        );

        // Note: compiler_fence is provided by core/sync/atomic.vr as a wrapper
        // that converts Ordering to Int. Don't register it here as an intrinsic.

        self.ctx.env.insert(
            verum_common::Text::from("verum_compiler_fence"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Int]), Type::Unit)),
        );

        // Atomic operation intrinsics (for sync/atomic.vr)
        // These are low-level FFI functions declared as extern in atomic.vr
        // but we register them here to ensure they're available during type checking

        // 32-bit atomics
        self.ctx.env.insert(
            verum_common::Text::from("verum_atomic_load_u32"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // ordering
                ]),
                Type::Int,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_atomic_store_u32"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // value
                    Type::Int, // ordering
                ]),
                Type::Unit,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_atomic_cas_u32"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // expected
                    Type::Int, // desired
                    Type::Int, // success_ordering
                    Type::Int, // failure_ordering
                ]),
                Type::Bool,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_atomic_fetch_add_u32"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // value
                    Type::Int, // ordering
                ]),
                Type::Int,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_atomic_fetch_sub_u32"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // value
                    Type::Int, // ordering
                ]),
                Type::Int,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_atomic_swap_u32"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // value
                    Type::Int, // ordering
                ]),
                Type::Int,
            )),
        );

        // 64-bit atomics
        self.ctx.env.insert(
            verum_common::Text::from("verum_atomic_load_u64"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // ordering
                ]),
                Type::Int,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_atomic_store_u64"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // value
                    Type::Int, // ordering
                ]),
                Type::Unit,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_atomic_cas_u64"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // expected
                    Type::Int, // desired
                    Type::Int, // success_ordering
                    Type::Int, // failure_ordering
                ]),
                Type::Bool,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_atomic_fetch_add_u64"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // value
                    Type::Int, // ordering
                ]),
                Type::Int,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_atomic_fetch_sub_u64"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // value
                    Type::Int, // ordering
                ]),
                Type::Int,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_atomic_swap_u64"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // value
                    Type::Int, // ordering
                ]),
                Type::Int,
            )),
        );

        // Bool atomics
        self.ctx.env.insert(
            verum_common::Text::from("verum_atomic_load_bool"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // ordering
                ]),
                Type::Int,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_atomic_store_bool"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // value
                    Type::Int, // ordering
                ]),
                Type::Unit,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_atomic_cas_bool"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // expected
                    Type::Int, // desired
                    Type::Int, // success_ordering
                    Type::Int, // failure_ordering
                ]),
                Type::Bool,
            )),
        );

        // Memory allocation intrinsics
        let alloc_t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("verum_alloc"),
            TypeScheme::poly(
                List::from_iter([alloc_t]),
                Type::function(
                    List::from_iter([Type::Int]), // size
                    Type::Reference {
                        inner: Box::new(Type::Var(alloc_t)),
                        mutable: true,
                    },
                ),
            ),
        );

        // Heap allocation intrinsic (used by collections/deque.vr)
        let heap_alloc_t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("__verum_heap_alloc"),
            TypeScheme::poly(
                List::from_iter([heap_alloc_t]),
                Type::function(
                    List::from_iter([Type::Int]), // size
                    Type::Reference {
                        inner: Box::new(Type::Var(heap_alloc_t)),
                        mutable: true,
                    },
                ),
            ),
        );

        let forget_t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("forget"),
            TypeScheme::poly(
                List::from_iter([forget_t]),
                Type::function(List::from_iter([Type::Var(forget_t)]), Type::Unit),
            ),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_time_sleep_ns"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Int]), Type::Unit)),
        );

        // Env intrinsics
        self.ctx.env.insert(
            verum_common::Text::from("arg"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Int]), Type::Text)),
        );

        // NOTE: __verum_env_get is registered from extern block in core/env.vr
        // Actual FFI signature: (name: &unsafe Byte, name_len: Int, buf: &unsafe Byte, buf_len: Int) -> Int
        // We don't register it here to avoid conflicts with extern block registration

        // verum_env_set is registered from the extern block in core/env.vr
        // Signature: (name: &Byte, name_len: Int, value: &Byte, value_len: Int) -> Int
        // NOTE: We don't register it here to avoid conflicts with extern block registration

        // NOTE: verum_exec_env_current is registered from extern block in async/executor.vr
        // We don't register it here to avoid conflicts with extern block registration

        // Print intrinsics (verum_ prefix versions used by stdlib)
        // These take raw byte pointer and length, not Text
        self.ctx.env.insert(
            verum_common::Text::from("verum_print"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // len
                ]),
                Type::Unit,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_println"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // len
                ]),
                Type::Unit,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_eprint"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // len
                ]),
                Type::Unit,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_eprintln"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::UnsafeReference {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int, // len
                ]),
                Type::Unit,
            )),
        );

        // Process control intrinsics
        self.ctx.env.insert(
            verum_common::Text::from("verum_abort"),
            TypeScheme::mono(Type::function(List::new(), Type::Never)),
        );

        self.ctx.env.insert(
            verum_common::Text::from("verum_exit"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Int]), // exit code
                Type::Never,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("__verum_exit"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Int]), // exit code
                Type::Never,
            )),
        );

        // Format args intrinsic (returns formatted string from format string and args)
        // This is a compiler magic function that handles format strings
        let format_args_t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("format_args"),
            TypeScheme::poly(
                List::from_iter([format_args_t]),
                Type::function(
                    List::from_iter([Type::Var(format_args_t)]), // Accept anything
                    Type::Text,
                ),
            ),
        );

        // NOTE: verum_env_remove is registered from extern block in core/env.vr
        // Actual FFI signature: (name: &Byte, name_len: Int) -> Int
        // We don't register it here to avoid conflicts with extern block registration

        // NOTE: verum_exec_env_fork is registered from extern block in async/executor.vr
        // We don't register it here to avoid conflicts with extern block registration

        // Boolean operators (as functions for pattern matching)
        self.ctx.env.insert(
            verum_common::Text::from("or"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Bool, Type::Bool]),
                Type::Bool,
            )),
        );

        self.ctx.env.insert(
            verum_common::Text::from("and"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Bool, Type::Bool]),
                Type::Bool,
            )),
        );

        // ============================================================
        // TLS INTRINSICS
        // ============================================================

        // tls_get_base() -> *mut T
        let tls_base_t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("tls_get_base"),
            TypeScheme::poly(
                List::from_iter([tls_base_t]),
                Type::function(
                    List::new(),
                    Type::Pointer {
                        inner: Box::new(Type::Var(tls_base_t)),
                        mutable: true,
                    },
                ),
            ),
        );

        // tls_slot_get(slot: UInt8) -> *const T
        let tls_get_t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("tls_slot_get"),
            TypeScheme::poly(
                List::from_iter([tls_get_t]),
                Type::function(
                    List::from_iter([Type::Int]),
                    Type::Pointer {
                        inner: Box::new(Type::Var(tls_get_t)),
                        mutable: false,
                    },
                ),
            ),
        );

        // tls_slot_set(slot: UInt8, value: *const T)
        let tls_set_t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("tls_slot_set"),
            TypeScheme::poly(
                List::from_iter([tls_set_t]),
                Type::function(
                    List::from_iter([
                        Type::Int,
                        Type::Pointer {
                            inner: Box::new(Type::Var(tls_set_t)),
                            mutable: false,
                        },
                    ]),
                    Type::Unit,
                ),
            ),
        );

        // tls_slot_clear(slot: UInt8)
        self.ctx.env.insert(
            verum_common::Text::from("tls_slot_clear"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Int]), Type::Unit)),
        );

        // tls_slot_has(slot: UInt8) -> Bool
        self.ctx.env.insert(
            verum_common::Text::from("tls_slot_has"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Int]), Type::Bool)),
        );

        // tls_frame_push() -> *const T
        let tls_frame_t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("tls_frame_push"),
            TypeScheme::poly(
                List::from_iter([tls_frame_t]),
                Type::function(
                    List::new(),
                    Type::Pointer {
                        inner: Box::new(Type::Var(tls_frame_t)),
                        mutable: false,
                    },
                ),
            ),
        );

        // tls_frame_pop()
        self.ctx.env.insert(
            verum_common::Text::from("tls_frame_pop"),
            TypeScheme::mono(Type::function(List::new(), Type::Unit)),
        );

        // tls_read_ptr<T>(offset: Int) -> *const T
        let tls_read_ptr_t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("tls_read_ptr"),
            TypeScheme::poly(
                List::from_iter([tls_read_ptr_t]),
                Type::function(
                    List::from_iter([Type::Int]),
                    Type::Pointer {
                        inner: Box::new(Type::Var(tls_read_ptr_t)),
                        mutable: false,
                    },
                ),
            ),
        );

        // tls_write_ptr<T>(offset: Int, value: *const T)
        let tls_write_ptr_t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("tls_write_ptr"),
            TypeScheme::poly(
                List::from_iter([tls_write_ptr_t]),
                Type::function(
                    List::from_iter([
                        Type::Int,
                        Type::Pointer {
                            inner: Box::new(Type::Var(tls_write_ptr_t)),
                            mutable: false,
                        },
                    ]),
                    Type::Unit,
                ),
            ),
        );

        // tls_read_i32(offset: Int) -> Int32
        self.ctx.env.insert(
            verum_common::Text::from("tls_read_i32"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Int]), Type::Int)),
        );

        // tls_write_i32(offset: Int, value: Int32)
        self.ctx.env.insert(
            verum_common::Text::from("tls_write_i32"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Int, Type::Int]),
                Type::Unit,
            )),
        );

        // tls_read_usize(offset: Int) -> Int
        self.ctx.env.insert(
            verum_common::Text::from("tls_read_usize"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Int]), Type::Int)),
        );

        // tls_write_usize(offset: Int, value: Int)
        self.ctx.env.insert(
            verum_common::Text::from("tls_write_usize"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Int, Type::Int]),
                Type::Unit,
            )),
        );

        // ============================================================
        // SYNC INTRINSICS
        // ============================================================

        // spin_hint()
        self.ctx.env.insert(
            verum_common::Text::from("spin_hint"),
            TypeScheme::mono(Type::function(List::new(), Type::Unit)),
        );

        // spin_loop_hint() (alias)
        self.ctx.env.insert(
            verum_common::Text::from("spin_loop_hint"),
            TypeScheme::mono(Type::function(List::new(), Type::Unit)),
        );

        // memory_fence(order: Int)
        self.ctx.env.insert(
            verum_common::Text::from("memory_fence"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Int]), Type::Unit)),
        );

        // atomic_fence(order: Int)
        self.ctx.env.insert(
            verum_common::Text::from("atomic_fence"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Int]), Type::Unit)),
        );

        // compiler_fence(order: Int)
        self.ctx.env.insert(
            verum_common::Text::from("compiler_fence"),
            TypeScheme::mono(Type::function(List::from_iter([Type::Int]), Type::Unit)),
        );

        // Ordering constants
        self.ctx.env.insert(
            verum_common::Text::from("ORDERING_RELAXED"),
            TypeScheme::mono(Type::Int),
        );
        self.ctx.env.insert(
            verum_common::Text::from("ORDERING_ACQUIRE"),
            TypeScheme::mono(Type::Int),
        );
        self.ctx.env.insert(
            verum_common::Text::from("ORDERING_RELEASE"),
            TypeScheme::mono(Type::Int),
        );
        self.ctx.env.insert(
            verum_common::Text::from("ORDERING_ACQ_REL"),
            TypeScheme::mono(Type::Int),
        );
        self.ctx.env.insert(
            verum_common::Text::from("ORDERING_SEQ_CST"),
            TypeScheme::mono(Type::Int),
        );

        // futex_wait(addr: *const UInt32, expected: UInt32, timeout_ns: UInt64) -> Int32
        self.ctx.env.insert(
            verum_common::Text::from("futex_wait"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::Pointer {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int,
                    Type::Int,
                ]),
                Type::Int,
            )),
        );

        // futex_wake(addr: *const UInt32, count: UInt32) -> Int32
        self.ctx.env.insert(
            verum_common::Text::from("futex_wake"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::Pointer {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int,
                ]),
                Type::Int,
            )),
        );

        // futex_wake_one(addr: *const UInt32) -> Int32
        self.ctx.env.insert(
            verum_common::Text::from("futex_wake_one"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Pointer {
                    inner: Box::new(Type::Int),
                    mutable: false,
                }]),
                Type::Int,
            )),
        );

        // futex_wake_all(addr: *const UInt32) -> Int32
        self.ctx.env.insert(
            verum_common::Text::from("futex_wake_all"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Pointer {
                    inner: Box::new(Type::Int),
                    mutable: false,
                }]),
                Type::Int,
            )),
        );

        // spinlock_try_lock(lock: *mut UInt32) -> Bool
        self.ctx.env.insert(
            verum_common::Text::from("spinlock_try_lock"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Pointer {
                    inner: Box::new(Type::Int),
                    mutable: true,
                }]),
                Type::Bool,
            )),
        );

        // spinlock_lock(lock: *mut UInt32)
        self.ctx.env.insert(
            verum_common::Text::from("spinlock_lock"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Pointer {
                    inner: Box::new(Type::Int),
                    mutable: true,
                }]),
                Type::Unit,
            )),
        );

        // spinlock_unlock(lock: *mut UInt32)
        self.ctx.env.insert(
            verum_common::Text::from("spinlock_unlock"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Pointer {
                    inner: Box::new(Type::Int),
                    mutable: true,
                }]),
                Type::Unit,
            )),
        );

        // spinlock_is_locked(lock: *const UInt32) -> Bool
        self.ctx.env.insert(
            verum_common::Text::from("spinlock_is_locked"),
            TypeScheme::mono(Type::function(
                List::from_iter([Type::Pointer {
                    inner: Box::new(Type::Int),
                    mutable: false,
                }]),
                Type::Bool,
            )),
        );

        // atomic_load_u32(addr: *const UInt32, ordering: Int) -> UInt32
        self.ctx.env.insert(
            verum_common::Text::from("atomic_load_u32"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::Pointer {
                        inner: Box::new(Type::Int),
                        mutable: false,
                    },
                    Type::Int,
                ]),
                Type::Int,
            )),
        );

        // atomic_store_u32(addr: *mut UInt32, value: UInt32, ordering: Int)
        self.ctx.env.insert(
            verum_common::Text::from("atomic_store_u32"),
            TypeScheme::mono(Type::function(
                List::from_iter([
                    Type::Pointer {
                        inner: Box::new(Type::Int),
                        mutable: true,
                    },
                    Type::Int,
                    Type::Int,
                ]),
                Type::Unit,
            )),
        );

        // monotonic_nanos() -> UInt64
        self.ctx.env.insert(
            verum_common::Text::from("monotonic_nanos"),
            TypeScheme::mono(Type::function(List::new(), Type::Int)),
        );
    }

    /// Register built-in types and functions.
    /// This is the full registration for normal build mode.
    /// For stdlib bootstrap mode, use `register_primitives()` instead.
    ///

    /// This should be called once when starting type checking
    /// Enable lenient context resolution where undefined contexts produce
    /// warnings instead of errors. Used during stdlib body checking where
    /// context declarations may not be visible to the type checker.
    pub fn set_lenient_contexts(&mut self, lenient: bool) {
        self.context_resolver.set_lenient_contexts(lenient);
        self.context_checker.set_lenient(lenient);
    }

    /// Toggle cubical-type normalization at the unification layer.
    ///

    /// Wired by the compiler's semantic-analysis phase from
    /// `[types] cubical` in `verum.toml`. When off, the unifier uses
    /// strict syntactic equality on Path / Partial / Eq endpoints,
    /// skipping the cubical `whnf` normalizer.
    pub fn set_cubical_enabled(&mut self, enabled: bool) {
        self.unifier.set_cubical_enabled(enabled);
    }

    /// Toggle dependent-type features (Pi, Sigma, dependent pattern
    /// matching). Wired from `[types] dependent` in `verum.toml`.
    /// When off, match expressions use regular (non-dependent)
    /// pattern matching even on inductive types with indices.
    pub fn set_dependent_enabled(&mut self, enabled: bool) {
        self.dependent_enabled = enabled;
    }

    pub fn set_higher_kinded_enabled(&mut self, enabled: bool) {
        self.higher_kinded_enabled = enabled;
    }

    /// Apply `[protocols].higher_kinded_protocols` to the type
    /// checker. When false (the default), protocol declarations
    /// that include an HKT generic parameter (e.g. `protocol
    /// Functor<F<_>>`) are rejected at registration time with
    /// `TypeError::Other` citing the manifest field.
    ///

    /// Manifest validation enforces that this field can be true
    /// only when `[types].higher_kinded` is also true.
    pub fn set_higher_kinded_protocols_enabled(&mut self, enabled: bool) {
        self.higher_kinded_protocols_enabled = enabled;
    }

    /// Read-only accessor — exposed for diagnostics + tests.
    #[inline]
    pub fn higher_kinded_protocols_enabled(&self) -> bool {
        self.higher_kinded_protocols_enabled
    }

    /// Apply `[protocols].generic_associated_types` to the type
    /// checker. When false (the default), associated-type
    /// declarations inside a protocol that include type
    /// parameters (`type Item<T>` — a GAT) are rejected at
    /// registration time with `TypeError::Other` citing the
    /// manifest field.
    pub fn set_generic_associated_types_enabled(&mut self, enabled: bool) {
        self.generic_associated_types_enabled = enabled;
    }

    /// Read-only accessor — exposed for diagnostics + tests.
    #[inline]
    pub fn generic_associated_types_enabled(&self) -> bool {
        self.generic_associated_types_enabled
    }

    /// MLS classification sidecar — set the classification level
    /// for a binding (#289 Phase 2b foundation).
    ///

    /// Phase 2b-Integration (separate follow-up) calls this from
    /// the parameter-introduction site in `synth_function_decl`
    /// to seed the sidecar with each parameter's
    /// `@classification` level. Subsequent let-binding sites
    /// (Phase 2b-Full) propagate by reading the source variable's
    /// classification and joining into the destination.
    pub fn set_binding_classification(
        &mut self,
        name: verum_common::Text,
        level: verum_common::mls::MlsLevel,
    ) {
        self.classification_map.insert(name, level);
    }

    /// Read the classification level of a binding (#289).
    ///

    /// Returns `MlsLevel::Public` (the safe default) for unknown
    /// bindings. Callers that need to distinguish "not classified"
    /// from "explicitly Public" should use `binding_classification_
    /// explicit` instead.
    #[inline]
    pub fn binding_classification(&self, name: &verum_common::Text) -> verum_common::mls::MlsLevel {
        self.classification_map
            .get(name)
            .copied()
            .unwrap_or(verum_common::mls::MlsLevel::Public)
    }

    /// Distinguishes "no entry" from "explicit Public" for callers
    /// that need to detect unclassified bindings (e.g. the Phase 3
    /// sink-detection gate which only fires on explicitly-classified
    /// values flowing into sinks).
    #[inline]
    pub fn binding_classification_explicit(
        &self,
        name: &verum_common::Text,
    ) -> Option<verum_common::mls::MlsLevel> {
        self.classification_map.get(name).copied()
    }

    /// Drain the classification sidecar — exposed for diagnostics
    /// (audit reports listing every classified binding) and for
    /// scope-exit cleanup.
    pub fn drain_classification_map(
        &mut self,
    ) -> std::collections::HashMap<verum_common::Text, verum_common::mls::MlsLevel> {
        std::mem::take(&mut self.classification_map)
    }

    /// Look up the parameter classification list for a registered
    /// function (#293). Returns `None` for unknown / unregistered
    /// functions; callers fall back to the no-classification path
    /// (treats every parameter as Public-required) in that case
    /// — keeps existing call sites for unannotated functions
    /// behaving identically.
    pub fn function_param_classifications(
        &self,
        function_name: &Text,
    ) -> Option<&List<verum_common::mls::MlsLevel>> {
        self.function_param_classifications.get(function_name)
    }

    /// Down-flow check for a single argument-to-parameter binding
    /// (#293). Returns `Ok(())` when the parameter's
    /// classification subsumes the argument's required protection
    /// level (`param >= arg`), else `Err(TypeError::Other)`
    /// citing the source / sink levels and the parameter index.
    ///

    /// **Lattice contract**: classification represents PROTECTION
    /// REQUIREMENTS. A Secret value requires Secret-level
    /// protection; the parameter declaration is the function's
    /// CONTRACT for what protection it provides. The function
    /// must provide AT LEAST the protection the data needs.
    ///

    /// Examples:
    ///  - arg=Public, param=Public → OK (no requirement)
    ///  - arg=Public, param=Secret → OK (over-protected)
    ///  - arg=Secret, param=Public → REJECT (Public
    ///  protection insufficient for Secret data — the leak
    ///  this gate catches)
    ///  - arg=Secret, param=Secret → OK (exact match)
    ///  - arg=TopSecret, param=Secret → REJECT
    ///  - arg=TopSecret, param=TopSecret → OK
    ///

    /// Phase 2b-FinalIntegration (separate task) calls this at
    /// synth_call / check_app sites for every argument; this
    /// commit lays the helper so the integration is just
    /// "iterate args + call helper".
    pub fn check_classification_downflow(
        &self,
        arg_level: verum_common::mls::MlsLevel,
        param_level: verum_common::mls::MlsLevel,
        function_name: &str,
        param_index: usize,
        param_name: &str,
    ) -> Result<()> {
        // The parameter must subsume (>=) the argument's
        // classification — the function provides at least the
        // protection the argument's data requires.
        if param_level.subsumes(arg_level) {
            return Ok(());
        }
        Err(TypeError::Other(verum_common::Text::from(format!(
            "MLS down-flow rejected: {}-classified argument cannot flow into \
             {}'s parameter `{}` (index {}) which provides only {} \
             protection.  Either elevate the parameter to \
             `@classification({})` (or higher), or wrap the call site in \
             `@declassify {{ … }}` to explicitly accept the leak.",
            arg_level.as_manifest_str(),
            function_name,
            param_name,
            param_index,
            param_level.as_manifest_str(),
            arg_level.as_manifest_str(),
        ))))
    }

    /// Walk a module and validate every call site's classification
    /// down-flow contract (#294). Returns the list of
    /// `TypeError::Other` diagnostics for every leak detected;
    /// the empty list means every call respected the contract.
    ///

    /// This is a separate gate from synth_call's main check loop —
    /// keeping it module-level + post-checking lets callers opt
    /// into MLS enforcement without invasive changes to the core
    /// type-checker dispatch. Embedders that want strict MLS run
    /// this as a phase after type-checking; embedders that don't
    /// need it (default Public-floor manifest) skip it for zero
    /// overhead.
    ///

    /// Coverage in this Phase 2b-Final-Integration:
    ///  - Top-level `Path(fn)(args)` calls in function bodies.
    ///  - Method calls (`x.method(args)`) — the receiver's
    ///  classification joins the args.
    /// Method dispatch fully + nested calls within complex
    /// expressions are #294-Followup.
    pub fn check_module_call_classifications(&self, module: &verum_ast::Module) -> Vec<TypeError> {
        let mut errors = Vec::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                // Phase 2b @declassify escape hatch (#295): when a
                // function carries the `@declassify` attribute, its
                // body is the explicit boundary where classified
                // data is allowed to flow into lower-classification
                // sinks. The user takes responsibility for the
                // declassification — the down-flow walker skips
                // call sites within this body entirely.
                //

                // Architectural rationale: classification is a
                // type-level safety property; declassification is a
                // type-level escape that the user EXPLICITLY opts
                // into per-function. The function itself is now the
                // declassification boundary — its callers see only
                // its return value, and the caller's @classification
                // (if any) governs whether the caller can in turn
                // be a declassification site for further uses.
                if has_declassify_attr_on_function(func) {
                    continue;
                }
                if let verum_common::Maybe::Some(body) = &func.body {
                    self.walk_body_for_call_classifications(body, &mut errors);
                }
            }
        }
        errors
    }

    fn walk_body_for_call_classifications(
        &self,
        body: &verum_ast::decl::FunctionBody,
        errors: &mut Vec<TypeError>,
    ) {
        match body {
            verum_ast::decl::FunctionBody::Block(blk) => {
                self.walk_block_for_call_classifications(blk, errors);
            }
            verum_ast::decl::FunctionBody::Expr(e) => {
                self.walk_expr_for_call_classifications(e, errors);
            }
        }
    }

    fn walk_block_for_call_classifications(
        &self,
        block: &verum_ast::expr::Block,
        errors: &mut Vec<TypeError>,
    ) {
        for stmt in &block.stmts {
            match &stmt.kind {
                verum_ast::stmt::StmtKind::Expr { expr, .. } => {
                    self.walk_expr_for_call_classifications(expr, errors);
                }
                verum_ast::stmt::StmtKind::Let { value, .. } => {
                    if let verum_common::Maybe::Some(v) = value {
                        self.walk_expr_for_call_classifications(v, errors);
                    }
                }
                _ => {}
            }
        }
        if let verum_common::Maybe::Some(e) = &block.expr {
            self.walk_expr_for_call_classifications(e, errors);
        }
    }

    fn walk_expr_for_call_classifications(
        &self,
        expr: &verum_ast::expr::Expr,
        errors: &mut Vec<TypeError>,
    ) {
        use verum_ast::expr::ExprKind;
        match &expr.kind {
            ExprKind::Call { func, args, .. } => {
                // Recurse first so nested calls are checked.
                self.walk_expr_for_call_classifications(func, errors);
                for a in args.iter() {
                    self.walk_expr_for_call_classifications(a, errors);
                }
                // Then check this call's down-flow.
                if let ExprKind::Path(path) = &func.kind {
                    if let Some(fn_ident) = path.as_ident() {
                        let fn_name = fn_ident.name.clone();
                        if let Some(param_classifications) =
                            self.function_param_classifications.get(&fn_name)
                        {
                            // Get parameter NAMES too for nicer
                            // diagnostics (when available).
                            let param_names = self.function_param_names.get(&fn_name).cloned();
                            for (i, arg) in args.iter().enumerate() {
                                let param_level = param_classifications
                                    .get(i)
                                    .copied()
                                    .unwrap_or(verum_common::mls::MlsLevel::Public);
                                let arg_level = self.expr_classification(arg);
                                let param_name = param_names
                                    .as_ref()
                                    .and_then(|names| names.get(i))
                                    .map(|t| t.as_str().to_string())
                                    .unwrap_or_else(|| format!("arg{}", i));
                                if let Err(e) = self.check_classification_downflow(
                                    arg_level,
                                    param_level,
                                    fn_name.as_str(),
                                    i,
                                    &param_name,
                                ) {
                                    errors.push(e);
                                }
                            }
                        }
                    }
                }
            }
            ExprKind::Binary { left, right, .. } => {
                self.walk_expr_for_call_classifications(left, errors);
                self.walk_expr_for_call_classifications(right, errors);
            }
            ExprKind::Unary { expr, .. } => {
                self.walk_expr_for_call_classifications(expr, errors);
            }
            ExprKind::Paren(inner) => {
                self.walk_expr_for_call_classifications(inner, errors);
            }
            ExprKind::Block(block) => {
                self.walk_block_for_call_classifications(block, errors);
            }
            // Other expression kinds: leaves or shapes the Phase
            // 2b-Final-Integration doesn't yet recurse into.
            // #294-Followup extends this to cover If / Match /
            // Lambda / Loop / etc.
            _ => {}
        }
    }

    /// Compute the MLS classification of an expression (#292
    /// propagation foundation).
    ///

    /// Walks common expression kinds applying the lattice's join:
    ///  - `Path(name)` → `binding_classification(name)`
    ///  - `Binary { left, right }` → `expr_classification(left).
    ///  join(right)` — both operands taint the result
    ///  - `Call { args }` → max of `expr_classification(arg)`
    ///  for each arg — function arguments propagate
    ///  - `Unary { expr }` → `expr_classification(expr)`
    ///  - `Paren(inner)` → `expr_classification(inner)`
    ///  - other kinds → `MlsLevel::Public` (no propagation in
    ///  Phase 2b foundation; `@declassify` blocks and explicit
    ///  classification escape hatches are #292-Followup)
    ///

    /// This is the load-bearing read site for the let-binding
    /// propagation pin: `let x = secret_param;` flows the param's
    /// classification onto `x`'s sidecar entry.
    pub fn expr_classification(&self, expr: &verum_ast::expr::Expr) -> verum_common::mls::MlsLevel {
        use verum_ast::expr::ExprKind;
        use verum_common::mls::MlsLevel;
        match &expr.kind {
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    return self.binding_classification(&ident.name);
                }
                MlsLevel::Public
            }
            ExprKind::Binary { left, right, .. } => self
                .expr_classification(left)
                .join(self.expr_classification(right)),
            ExprKind::Unary { expr, .. } => self.expr_classification(expr),
            ExprKind::Paren(inner) => self.expr_classification(inner),
            ExprKind::Call { func, args, .. } => {
                let mut acc = self.expr_classification(func);
                for a in args.iter() {
                    acc = acc.join(self.expr_classification(a));
                }
                acc
            }
            // Other kinds default to Public; the lattice's identity
            // means this is a no-op at downstream join sites.
            _ => MlsLevel::Public,
        }
    }

    pub fn set_universe_poly_enabled(&mut self, enabled: bool) {
        self.universe_poly_enabled = enabled;
    }

    pub fn set_coinductive_enabled(&mut self, enabled: bool) {
        self.coinductive_enabled = enabled;
    }

    pub fn set_quotient_enabled(&mut self, enabled: bool) {
        self.quotient_enabled = enabled;
    }

    /// Apply `[types].instance_search` to both the TypeChecker
    /// (where downstream type-system flow may consult it) AND the
    /// embedded `ProtocolChecker.instance_search_enabled` field
    /// where `find_impl` actually gates the Stage-2 generic-
    /// candidate scan. Closes the inert-defense pattern around
    /// the field — pre-fix only the type-checker store happened
    /// here, so even when the manifest disabled instance search
    /// the resolver still ran the full multi-stage candidate scan.
    pub fn set_instance_search_enabled(&mut self, enabled: bool) {
        self.instance_search_enabled = enabled;
        self.protocol_checker
            .write()
            .set_instance_search_enabled(enabled);
    }

    pub fn set_coherence_check_depth(&mut self, depth: u32) {
        self.coherence_check_depth = depth;
    }

    /// Apply `[protocols].resolution_strategy` to the embedded
    /// `ProtocolChecker`. Threads from manifest →
    /// `phase_checker.set_protocol_resolution_strategy(...)` →
    /// `ProtocolChecker.resolution_strategy`, which `find_impl`
    /// consults when multiple candidates are available. Closes the
    /// inert-defense pattern around the field — pre-fix the
    /// resolver hardcoded "most_specific" regardless of manifest.
    pub fn set_protocol_resolution_strategy(&mut self, strategy: impl Into<verum_common::Text>) {
        self.protocol_checker
            .write()
            .set_resolution_strategy(strategy);
    }

    /// Apply `[protocols].blanket_impls` to the embedded
    /// `ProtocolChecker`. When false, `find_impl` excludes
    /// candidates whose `for_type` is a bare type variable.
    pub fn set_protocol_blanket_impls(&mut self, allowed: bool) {
        self.protocol_checker.write().set_blanket_impls(allowed);
    }

    /// Apply `[protocols].coherence` to the embedded
    /// `ProtocolChecker`. Threads from manifest →
    /// `phase_checker.set_protocol_coherence_mode(...)` →
    /// `ProtocolChecker.coherence_mode`, which `register_impl`
    /// consults to gate orphan-rule + overlap checks. Closes the
    /// inert-defense pattern at session.rs:587 — pre-fix the
    /// production resolver always rejected orphan/overlap
    /// regardless of manifest.
    pub fn set_protocol_coherence_mode(&mut self, mode: crate::protocol::CoherenceMode) {
        self.protocol_checker.write().set_coherence_mode(mode);
    }

    /// Drain coherence violations the embedded `ProtocolChecker`
    /// downgraded to warnings under `CoherenceMode::Lenient`.
    /// Returns an empty vec under Strict / Off.
    pub fn drain_protocol_coherence_warnings(&mut self) -> Vec<crate::protocol::CoherenceError> {
        self.protocol_checker.write().drain_coherence_warnings()
    }

    pub fn register_builtins(&mut self) {
        // ============================================================
        // UNIFIED BUILTIN REGISTRATION
        //

        // All modes now use the same primitive registration.
        // Stdlib types (List, Map, Maybe, Result, etc.) come from stdlib .vr files.
        // Their methods and constructors are registered via implement blocks
        // in core/ .vr source files, NOT hardcoded here.
        // ============================================================
        self.register_primitives();
        self.register_intrinsics();
        self.register_meta_types();
        self.register_meta_builtins();
        // Register CBGR type aliases (RawPtr, Epoch, u32) — these are compiler
        // intrinsic types used by core/ stdlib files, not user-defined types.
        self.ctx.add_cbgr_type_aliases();
    }

    /// Register true compiler intrinsics that cannot be defined in stdlib.
    /// These require compiler-level support (source location, never-return, polymorphic output, etc.)
    fn register_intrinsics(&mut self) {
        // ============================================================
        // BUILT-IN FUNCTIONS (always registered)
        // These are compiler intrinsics that cannot be defined in stdlib.
        // ============================================================

        // Register print function: fn<T>(T) -> Unit
        // Accepts any type for convenience in tests
        let print_t = TypeVar::fresh();
        let print_params = List::from_iter([Type::Var(print_t)]);
        let print_ty = Type::function(print_params, Type::unit());
        let print_scheme = TypeScheme::poly(List::from_iter([print_t]), print_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("print"), print_scheme);

        // Register println function: fn<T>(T) -> Unit
        // Accepts any type for convenience in tests
        let println_t = TypeVar::fresh();
        let println_params = List::from_iter([Type::Var(println_t)]);
        let println_ty = Type::function(println_params, Type::unit());
        let println_scheme = TypeScheme::poly(List::from_iter([println_t]), println_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("println"), println_scheme);

        // Register format function: fn<T>(Text, ...) -> Text
        // Verum's format() takes a format string and variadic args
        let format_t = TypeVar::fresh();
        let format_params = List::from_iter([Type::Var(format_t)]);
        let format_ty = Type::function(format_params, Type::text());
        let format_scheme = TypeScheme::poly(List::from_iter([format_t]), format_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("format"), format_scheme);

        // Register assert function: fn assert(Bool) -> Unit
        // Also register with optional message: fn assert(Bool, Text) -> Unit
        // Note: We use a special builtin marker to allow both 1 and 2 arguments
        let assert_params = List::from_iter([Type::bool()]);
        let assert_ty = Type::function(assert_params, Type::unit());
        self.ctx.env.insert_mono("assert", assert_ty.clone());

        // Register assert with message variant: fn assert_msg(Bool, &Text) -> Unit
        // Note: Takes &Text to match stdlib definition in core/panic.vr
        let assert_with_msg_params = List::from_iter([
            Type::bool(),
            Type::Reference {
                inner: Box::new(Type::text()),
                mutable: false,
            },
        ]);
        let assert_with_msg_ty = Type::function(assert_with_msg_params, Type::unit());
        self.ctx.env.insert_mono("assert_msg", assert_with_msg_ty);

        // Register debug_assert function: fn debug_assert(Bool) -> Unit
        let debug_assert_params = List::from_iter([Type::bool()]);
        let debug_assert_ty = Type::function(debug_assert_params, Type::unit());
        self.ctx.env.insert_mono("debug_assert", debug_assert_ty);

        // Register debug_assert with message variant: fn debug_assert_msg(Bool, &Text) -> Unit
        // Note: Takes &Text to match stdlib definition in core/panic.vr
        let debug_assert_with_msg_params = List::from_iter([
            Type::bool(),
            Type::Reference {
                inner: Box::new(Type::text()),
                mutable: false,
            },
        ]);
        let debug_assert_with_msg_ty = Type::function(debug_assert_with_msg_params, Type::unit());
        self.ctx
            .env
            .insert_mono("debug_assert_msg", debug_assert_with_msg_ty);

        // Register assert_eq function: fn assert_eq<T>(T, T) -> Unit
        let assert_eq_t = TypeVar::fresh();
        let assert_eq_params = List::from_iter([Type::Var(assert_eq_t), Type::Var(assert_eq_t)]);
        let assert_eq_ty = Type::function(assert_eq_params, Type::unit());
        let assert_eq_scheme = TypeScheme::poly(List::from_iter([assert_eq_t]), assert_eq_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("assert_eq"), assert_eq_scheme);

        // Register assert_eq with message: fn assert_eq<T>(T, T, &Text) -> Unit
        // Note: Takes &Text to match stdlib definition in core/panic.vr
        let assert_eq_msg_t = TypeVar::fresh();
        let assert_eq_msg_params = List::from_iter([
            Type::Var(assert_eq_msg_t),
            Type::Var(assert_eq_msg_t),
            Type::Reference {
                inner: Box::new(Type::text()),
                mutable: false,
            },
        ]);
        let assert_eq_msg_ty = Type::function(assert_eq_msg_params, Type::unit());
        let assert_eq_msg_scheme =
            TypeScheme::poly(List::from_iter([assert_eq_msg_t]), assert_eq_msg_ty);
        self.ctx.env.insert(
            verum_common::Text::from("assert_eq_msg"),
            assert_eq_msg_scheme,
        );

        // Register assert_ne function: fn assert_ne<T>(T, T) -> Unit
        let assert_ne_t = TypeVar::fresh();
        let assert_ne_params = List::from_iter([Type::Var(assert_ne_t), Type::Var(assert_ne_t)]);
        let assert_ne_ty = Type::function(assert_ne_params, Type::unit());
        let assert_ne_scheme = TypeScheme::poly(List::from_iter([assert_ne_t]), assert_ne_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("assert_ne"), assert_ne_scheme);

        // Register assert_ne with message: fn assert_ne<T>(T, T, &Text) -> Unit
        // Note: Takes &Text to match stdlib definition in core/panic.vr
        let assert_ne_msg_t = TypeVar::fresh();
        let assert_ne_msg_params = List::from_iter([
            Type::Var(assert_ne_msg_t),
            Type::Var(assert_ne_msg_t),
            Type::Reference {
                inner: Box::new(Type::text()),
                mutable: false,
            },
        ]);
        let assert_ne_msg_ty = Type::function(assert_ne_msg_params, Type::unit());
        let assert_ne_msg_scheme =
            TypeScheme::poly(List::from_iter([assert_ne_msg_t]), assert_ne_msg_ty);
        self.ctx.env.insert(
            verum_common::Text::from("assert_ne_msg"),
            assert_ne_msg_scheme,
        );

        // Register assert_panics: fn assert_panics<T>(fn() -> T) -> Unit
        // Tests that a closure panics
        let assert_panics_t = TypeVar::fresh();
        let assert_panics_closure = Type::function(List::new(), Type::Var(assert_panics_t));
        let assert_panics_params = List::from_iter([assert_panics_closure]);
        let assert_panics_ty = Type::function(assert_panics_params, Type::unit());
        let assert_panics_scheme =
            TypeScheme::poly(List::from_iter([assert_panics_t]), assert_panics_ty);
        self.ctx.env.insert(
            verum_common::Text::from("assert_panics"),
            assert_panics_scheme,
        );

        // Register watch_channel: fn watch_channel<T>(T) -> (WatchSender<T>, WatchReceiver<T>)
        let watch_t = TypeVar::fresh();
        let watch_sender = Type::Generic {
            name: verum_common::Text::from("WatchSender"),
            args: List::from_iter([Type::Var(watch_t)]),
        };
        let watch_receiver = Type::Generic {
            name: verum_common::Text::from("WatchReceiver"),
            args: List::from_iter([Type::Var(watch_t)]),
        };
        let watch_return = Type::Tuple(List::from_iter([watch_sender, watch_receiver]));
        let watch_ty = Type::function(List::from_iter([Type::Var(watch_t)]), watch_return);
        let watch_scheme = TypeScheme::poly(List::from_iter([watch_t]), watch_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("watch_channel"), watch_scheme);

        // Register panic function: fn panic(&Text) -> Never
        // Note: Takes &Text to match stdlib definition in core/panic.vr
        let panic_params = List::from_iter([Type::Reference {
            inner: Box::new(Type::text()),
            mutable: false,
        }]);
        let panic_ty = Type::function(panic_params, Type::never());
        self.ctx.env.insert_mono("panic", panic_ty);

        // Register drop function: fn drop<T>(T) -> Unit
        // Takes ownership of a value and drops it
        let drop_t = TypeVar::fresh();
        let drop_params = List::from_iter([Type::Var(drop_t)]);
        let drop_ty = Type::function(drop_params, Type::unit());
        let drop_scheme = TypeScheme::poly(List::from_iter([drop_t]), drop_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("drop"), drop_scheme);

        // Register channel function: fn channel<T>() -> (Sender<T>, Receiver<T>)
        // Creates a multi-producer single-consumer channel
        let channel_t = TypeVar::fresh();
        let channel_t_type = Type::Var(channel_t);
        let sender_type = Type::Generic {
            name: verum_common::Text::from("Sender"),
            args: List::from_iter([channel_t_type.clone()]),
        };
        let receiver_type = Type::Generic {
            name: verum_common::Text::from("Receiver"),
            args: List::from_iter([channel_t_type]),
        };
        let channel_return_ty = Type::Tuple(List::from_iter([sender_type, receiver_type]));
        let channel_ty = Type::function(List::new(), channel_return_ty);
        let channel_scheme = TypeScheme::poly(List::from_iter([channel_t]), channel_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("channel"), channel_scheme);

        // Register transmute function: unsafe fn transmute<T, U>(T) -> U
        // Unsafe type-level cast
        let transmute_t = TypeVar::fresh();
        let transmute_u = TypeVar::fresh();
        let transmute_params = List::from_iter([Type::Var(transmute_t)]);
        let transmute_ty = Type::function(transmute_params, Type::Var(transmute_u));
        let transmute_scheme =
            TypeScheme::poly(List::from_iter([transmute_t, transmute_u]), transmute_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("transmute"), transmute_scheme);

        // Heap builtins
        let infinite_loop_t_var = TypeVar::fresh();
        let infinite_loop_t = Type::Var(infinite_loop_t_var);
        let builtin_infinite_loop_ty = Type::function(List::new(), infinite_loop_t);
        let builtin_infinite_loop_scheme = TypeScheme::poly(
            List::from_iter([infinite_loop_t_var]),
            builtin_infinite_loop_ty,
        );
        self.ctx.env.insert(
            verum_common::Text::from("builtin_infinite_loop"),
            builtin_infinite_loop_scheme,
        );

        // NOTE: size_of<T>() and align_of<T>() are DEPRECATED.
        // Use Type Properties instead: T.size, T.alignment
        // Comprehension expressions: "[expr for x in iter if cond]" list comprehension syntax

        // offset_of(type_name: Text, field_name: Text) -> Int
        // Returns the byte offset of a field within a struct.
        // Usage: offset_of("MyStruct", "field_name")
        // Note: This is a meta function, not a Type Property, because it needs a field parameter.
        // Alternatively can be called as: offset_of(MyStruct, field) where both are identifiers
        let offset_of_params =
            List::from_iter([Type::Var(TypeVar::fresh()), Type::Var(TypeVar::fresh())]);
        let offset_of_ty = Type::function(offset_of_params, Type::Int);
        let offset_of_scheme = TypeScheme::mono(offset_of_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("offset_of"), offset_of_scheme);

        // static_assert(condition: Bool) -> Unit
        // Compile-time assertion (used in FFI/layout tests)
        let static_assert_params = List::from_iter([Type::bool()]);
        let static_assert_ty = Type::function(static_assert_params, Type::unit());
        self.ctx.env.insert_mono("static_assert", static_assert_ty);

        // static_assert with message: static_assert(Bool, Text) -> Unit
        let static_assert_msg_params = List::from_iter([Type::bool(), Type::text()]);
        let static_assert_msg_ty = Type::function(static_assert_msg_params, Type::unit());
        self.ctx
            .env
            .insert_mono("static_assert_msg", static_assert_msg_ty);

        // properties_of(fn) -> PropertySet (for compile-time verification)
        // Returns the computational properties of a function
        {
            let property_set_ty = Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    "PropertySet",
                    Span::default(),
                )),
                args: List::new(),
            };
            // PropertySet type with .contains() method
            let contains_params = List::from_iter([property_set_ty.clone(), Type::Unknown]);
            let contains_ty = Type::function(contains_params, Type::bool());
            // Register PropertySet.contains method via type scheme
            self.ctx
                .env
                .insert_mono("__PropertySet_contains", contains_ty);

            // Register properties_of as fn(any) -> PropertySet
            let props_of_var = TypeVar::fresh();
            let props_of_params = List::from_iter([Type::Var(props_of_var)]);
            let props_of_ty = Type::function(props_of_params, property_set_ty);
            self.ctx.env.insert_mono("properties_of", props_of_ty);
        }

        // Computational property names as values (for use with properties_of)
        for prop_name in [
            "IO",
            "Async",
            "Fallible",
            "Mutates",
            "Spawns",
            "Allocates",
            "Pure",
            "Divergent",
        ] {
            let prop_ty = Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                    prop_name,
                    Span::default(),
                )),
                args: List::new(),
            };
            self.ctx.env.insert_mono(prop_name, prop_ty);
        }

        // unreachable() -> Never
        let unreachable_ty = Type::function(List::new(), Type::never());
        self.ctx.env.insert_mono("unreachable", unreachable_ty);

        // unimplemented() -> Never
        let unimplemented_ty = Type::function(List::new(), Type::never());
        self.ctx.env.insert_mono("unimplemented", unimplemented_ty);

        // todo() -> Never
        let todo_ty = Type::function(List::new(), Type::never());
        self.ctx.env.insert_mono("todo", todo_ty);

        // type_name<T>() -> Text
        let type_name_t = TypeVar::fresh();
        let type_name_ty = Type::function(List::new(), Type::text());
        let type_name_scheme = TypeScheme::poly(List::from_iter([type_name_t]), type_name_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("type_name"), type_name_scheme);

        // size_of<T>() -> Int (kept for compat even though deprecated)
        let size_of_t = TypeVar::fresh();
        let size_of_ty = Type::function(List::new(), Type::Int);
        let size_of_scheme = TypeScheme::poly(List::from_iter([size_of_t]), size_of_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("size_of"), size_of_scheme);

        // align_of<T>() -> Int (kept for compat even though deprecated)
        let align_of_t = TypeVar::fresh();
        let align_of_ty = Type::function(List::new(), Type::Int);
        let align_of_scheme = TypeScheme::poly(List::from_iter([align_of_t]), align_of_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("align_of"), align_of_scheme);

        // len<T>(T) -> Int — generic length function
        let len_t = TypeVar::fresh();
        let len_params = List::from_iter([Type::Var(len_t)]);
        let len_ty = Type::function(len_params, Type::Int);
        let len_scheme = TypeScheme::poly(List::from_iter([len_t]), len_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("len"), len_scheme);

        // drop<T>(T) -> Unit — explicit drop/deallocate
        let drop_t = TypeVar::fresh();
        let drop_params = List::from_iter([Type::Var(drop_t)]);
        let drop_ty = Type::function(drop_params, Type::unit());
        let drop_scheme = TypeScheme::poly(List::from_iter([drop_t]), drop_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("drop"), drop_scheme);

        // writeln<T>(T) -> Unit — write with newline (common in tests)
        let writeln_t = TypeVar::fresh();
        let writeln_params = List::from_iter([Type::Var(writeln_t)]);
        let writeln_ty = Type::function(writeln_params, Type::unit());
        let writeln_scheme = TypeScheme::poly(List::from_iter([writeln_t]), writeln_ty);
        self.ctx
            .env
            .insert(verum_common::Text::from("writeln"), writeln_scheme);

        // char module — register as a type variable to avoid "unbound variable: char"
        let char_t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("char"),
            TypeScheme::mono(Type::Var(char_t)),
        );

        // std module stub — many tests use std.sync.Mutex, std.collections, etc.
        let std_t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("std"),
            TypeScheme::mono(Type::Var(std_t)),
        );

        // fs module — filesystem operations
        let fs_t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("fs"),
            TypeScheme::mono(Type::Var(fs_t)),
        );

        // thread module — threading primitives
        let thread_t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("thread"),
            TypeScheme::mono(Type::Var(thread_t)),
        );

        // runtime module — runtime operations
        let runtime_t = TypeVar::fresh();
        self.ctx.env.insert(
            verum_common::Text::from("runtime"),
            TypeScheme::mono(Type::Var(runtime_t)),
        );

        // Common async/concurrency builtins used in L2 tests
        // Register with correct parameter counts for each function

        // Single-param async functions: fn(T) -> R
        for name in &[
            "join_all",
            "try_join_all",
            "join_all_settled",
            "spawn_blocking",
            "thread_rng",
            "sleep",
            "interval",
        ] {
            let t = TypeVar::fresh();
            let params = List::from_iter([Type::Var(t)]);
            let ret = TypeVar::fresh();
            let fn_ty = Type::function(params, Type::Var(ret));
            let scheme = TypeScheme::poly(List::from_iter([t, ret]), fn_ty);
            self.ctx.env.insert(verum_common::Text::from(*name), scheme);
        }

        // Two-param async functions: fn(A, B) -> R
        for name in &[
            "timeout",
            "timeout_at",
            "race",
            "try_join",
            "spawn_with_config",
        ] {
            let a = TypeVar::fresh();
            let b = TypeVar::fresh();
            let ret = TypeVar::fresh();
            let fn_ty = Type::function(
                List::from_iter([Type::Var(a), Type::Var(b)]),
                Type::Var(ret),
            );
            let scheme = TypeScheme::poly(List::from_iter([a, b, ret]), fn_ty);
            self.ctx.env.insert(verum_common::Text::from(*name), scheme);
        }

        // Three-param async functions: fn(A, B, C) -> R
        {
            let name = &"timeout_or_else";
            let a = TypeVar::fresh();
            let b = TypeVar::fresh();
            let c = TypeVar::fresh();
            let ret = TypeVar::fresh();
            let fn_ty = Type::function(
                List::from_iter([Type::Var(a), Type::Var(b), Type::Var(c)]),
                Type::Var(ret),
            );
            let scheme = TypeScheme::poly(List::from_iter([a, b, c, ret]), fn_ty);
            self.ctx.env.insert(verum_common::Text::from(*name), scheme);
        }

        // Variadic/special async functions
        // select is parsed as a special expression, not a function call - no registration needed
        // spawn is also parsed as a special expression (keyword)

        // Channel constructors: fn(T) -> (Sender<T>, Receiver<T>)
        for name in &["bounded_channel", "unbounded_channel", "oneshot_channel"] {
            let t = TypeVar::fresh();
            let params = List::from_iter([Type::Var(t)]);
            let ret = TypeVar::fresh();
            let fn_ty = Type::function(params, Type::Var(ret));
            let scheme = TypeScheme::poly(List::from_iter([t, ret]), fn_ty);
            self.ctx.env.insert(verum_common::Text::from(*name), scheme);
        }

        // join builtins with specific arities
        {
            // join2(a, b) -> (A, B)
            let a = TypeVar::fresh();
            let b = TypeVar::fresh();
            let ret = TypeVar::fresh();
            let fn_ty = Type::function(
                List::from_iter([Type::Var(a), Type::Var(b)]),
                Type::Var(ret),
            );
            self.ctx.env.insert(
                verum_common::Text::from("join2"),
                TypeScheme::poly(List::from_iter([a, b, ret]), fn_ty),
            );

            // join3(a, b, c) -> (A, B, C)
            let a = TypeVar::fresh();
            let b = TypeVar::fresh();
            let c = TypeVar::fresh();
            let ret = TypeVar::fresh();
            let fn_ty = Type::function(
                List::from_iter([Type::Var(a), Type::Var(b), Type::Var(c)]),
                Type::Var(ret),
            );
            self.ctx.env.insert(
                verum_common::Text::from("join3"),
                TypeScheme::poly(List::from_iter([a, b, c, ret]), fn_ty),
            );

            // join5(a, b, c, d, e) -> (A, B, C, D, E)
            let a = TypeVar::fresh();
            let b = TypeVar::fresh();
            let c = TypeVar::fresh();
            let d = TypeVar::fresh();
            let e = TypeVar::fresh();
            let ret = TypeVar::fresh();
            let fn_ty = Type::function(
                List::from_iter([
                    Type::Var(a),
                    Type::Var(b),
                    Type::Var(c),
                    Type::Var(d),
                    Type::Var(e),
                ]),
                Type::Var(ret),
            );
            self.ctx.env.insert(
                verum_common::Text::from("join5"),
                TypeScheme::poly(List::from_iter([a, b, c, d, e, ret]), fn_ty),
            );

            // try_join3(a, b, c) -> Result<(A, B, C), E>
            let a = TypeVar::fresh();
            let b = TypeVar::fresh();
            let c = TypeVar::fresh();
            let ret = TypeVar::fresh();
            let fn_ty = Type::function(
                List::from_iter([Type::Var(a), Type::Var(b), Type::Var(c)]),
                Type::Var(ret),
            );
            self.ctx.env.insert(
                verum_common::Text::from("try_join3"),
                TypeScheme::poly(List::from_iter([a, b, c, ret]), fn_ty),
            );

            // race2(a, b) -> R
            let a = TypeVar::fresh();
            let b = TypeVar::fresh();
            let ret = TypeVar::fresh();
            let fn_ty = Type::function(
                List::from_iter([Type::Var(a), Type::Var(b)]),
                Type::Var(ret),
            );
            self.ctx.env.insert(
                verum_common::Text::from("race2"),
                TypeScheme::poly(List::from_iter([a, b, ret]), fn_ty),
            );

            // race3(a, b, c) -> R
            let a = TypeVar::fresh();
            let b = TypeVar::fresh();
            let c = TypeVar::fresh();
            let ret = TypeVar::fresh();
            let fn_ty = Type::function(
                List::from_iter([Type::Var(a), Type::Var(b), Type::Var(c)]),
                Type::Var(ret),
            );
            self.ctx.env.insert(
                verum_common::Text::from("race3"),
                TypeScheme::poly(List::from_iter([a, b, c, ret]), fn_ty),
            );

            // race_all(list) -> R
            let a = TypeVar::fresh();
            let ret = TypeVar::fresh();
            let fn_ty = Type::function(List::from_iter([Type::Var(a)]), Type::Var(ret));
            self.ctx.env.insert(
                verum_common::Text::from("race_all"),
                TypeScheme::poly(List::from_iter([a, ret]), fn_ty),
            );

            // set_panic_hook(fn) -> ()
            let a = TypeVar::fresh();
            let fn_ty = Type::function(List::from_iter([Type::Var(a)]), Type::unit());
            self.ctx.env.insert(
                verum_common::Text::from("set_panic_hook"),
                TypeScheme::poly(List::from_iter([a]), fn_ty),
            );

            // oneshot() -> (Sender<T>, Receiver<T>)
            let ret = TypeVar::fresh();
            let fn_ty = Type::function(List::new(), Type::Var(ret));
            self.ctx.env.insert(
                verum_common::Text::from("oneshot"),
                TypeScheme::poly(List::from_iter([ret]), fn_ty),
            );
        }

        // Common type constructors used in L2 tests
        // NOTE: Do NOT add context names (Database, Logger, Analytics, Metrics)
        // as they need to be resolved through the context system, not as builtins.
        for name in &[
            "Client",
            "SupervisorStrategy",
            "PriorityQueue",
            "DateTime",
            "Utc",
            "FixedOffset",
            "NaiveDate",
            "Duration",
            "Sha",
            "Aes",
            "Md",
            "Hmac",
            "StdRng",
            "SaltString",
            "RegexBuilder",
            "FuturesUnordered",
            "ResolverOpts",
            "Level",
            "LazyStatic",
            "Deque",
            "BTreeMap",
            "BTreeSet",
            "LinkedList",
            "Instant",
            "SystemTime",
        ] {
            let t = TypeVar::fresh();
            self.ctx.env.insert(
                verum_common::Text::from(*name),
                TypeScheme::mono(Type::Var(t)),
            );
        }

        // ============================================================
        // CUBICAL TYPE THEORY BUILT-INS (Phase B)
        //

        // These are the core cubical primitives. They're resolved by name
        // during type checking and lowered to CubicalTerm operations in
        // the cubical normalizer (crates/verum_types/src/cubical.rs).
        //

        // In Verum's 3-keyword philosophy, these are NOT keywords but
        // context-sensitive built-in names (like Path in the type parser).
        // ============================================================

        // transport<A: fn(I) -> Type>(p: I, x: A(i0)) -> A(p)
        // Transports a value along a path of types.
        {
            let tv_a = TypeVar::fresh();
            let tv_i = TypeVar::fresh();
            let params = List::from_iter([Type::Var(tv_i), Type::Var(tv_a)]);
            let ret = Type::Var(tv_a);
            let ty = Type::function(params, ret);
            self.ctx.env.insert(
                verum_common::Text::from("transport"),
                TypeScheme::poly(List::from_iter([tv_a, tv_i]), ty),
            );
        }

        // hcomp<A: Type>(base: A, sides: fn(I) -> A) -> A
        // Homogeneous composition: fills a cube from its base and sides.
        {
            let tv_a = TypeVar::fresh();
            let params = List::from_iter([Type::Var(tv_a), Type::Var(tv_a)]);
            let ret = Type::Var(tv_a);
            let ty = Type::function(params, ret);
            self.ctx.env.insert(
                verum_common::Text::from("hcomp"),
                TypeScheme::poly(List::from_iter([tv_a]), ty),
            );
        }

        // Interval, i0, i1 — cubical interval type and its endpoints.
        // Registered as opaque types (fresh type variables) that unify
        // with whatever the cubical normalizer expects.
        {
            let interval_tv = TypeVar::fresh();
            self.ctx.env.insert(
                verum_common::Text::from("Interval"),
                TypeScheme::mono(Type::Var(interval_tv)),
            );
            self.ctx.env.insert(
                verum_common::Text::from("i0"),
                TypeScheme::mono(Type::Var(interval_tv)),
            );
            self.ctx.env.insert(
                verum_common::Text::from("i1"),
                TypeScheme::mono(Type::Var(interval_tv)),
            );
        }
    }

    /// Register meta system types needed by the compiler.
    /// These are types used by the meta/macro system (TokenStream, Ident, etc.)
    /// and generic type constructors (List, Map, Set) needed before stdlib is loaded.
    fn register_meta_types(&mut self) {
        // ============================================================
        // META SYSTEM TYPES AND BUILTINS
        // Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Compile-time metaprogramming
        // ============================================================

        // TokenStream - sequence of tokens for quote/splice
        let token_stream_ty = Type::Generic {
            name: verum_common::Text::from("TokenStream"),
            args: List::new(),
        };
        self.ctx.define_type(
            verum_common::Text::from("TokenStream"),
            token_stream_ty.clone(),
        );
        self.ctx.env.insert(
            verum_common::Text::from("TokenStream"),
            TypeScheme::mono(token_stream_ty.clone()),
        );

        // Ident - identifier for hygiene
        let ident_ty = Type::Generic {
            name: verum_common::Text::from("Ident"),
            args: List::new(),
        };
        self.ctx
            .define_type(verum_common::Text::from("Ident"), ident_ty.clone());
        self.ctx.env.insert(
            verum_common::Text::from("Ident"),
            TypeScheme::mono(ident_ty.clone()),
        );

        // TypeInfo - type reflection information
        let type_info_ty = Type::Generic {
            name: verum_common::Text::from("TypeInfo"),
            args: List::new(),
        };
        self.ctx
            .define_type(verum_common::Text::from("TypeInfo"), type_info_ty.clone());
        self.ctx.env.insert(
            verum_common::Text::from("TypeInfo"),
            TypeScheme::mono(type_info_ty.clone()),
        );

        // UInt - unsigned integer
        let uint_ty = Type::Generic {
            name: verum_common::Text::from("UInt"),
            args: List::new(),
        };
        self.ctx
            .define_type(verum_common::Text::from("UInt"), uint_ty.clone());
        self.ctx.env.insert(
            verum_common::Text::from("UInt"),
            TypeScheme::mono(uint_ty.clone()),
        );

        // Bytes - byte sequence
        let bytes_ty = Type::Generic {
            name: verum_common::Text::from("Bytes"),
            args: List::new(),
        };
        self.ctx
            .define_type(verum_common::Text::from("Bytes"), bytes_ty.clone());
        self.ctx.env.insert(
            verum_common::Text::from("Bytes"),
            TypeScheme::mono(bytes_ty.clone()),
        );

        // ============================================================
        // META CONTEXT PROTOCOLS
        // These are valid context types for meta functions
        // ============================================================

        // Register meta contexts as valid context protocols
        self.context_resolver
            .register_protocol_as_context(verum_common::Text::from("MetaTypes"));
        self.context_resolver
            .register_protocol_as_context(verum_common::Text::from("MetaRuntime"));
        self.context_resolver
            .register_protocol_as_context(verum_common::Text::from("BuildAssets"));
        self.context_resolver
            .register_protocol_as_context(verum_common::Text::from("CompileDiag"));
        self.context_resolver
            .register_protocol_as_context(verum_common::Text::from("AstAccess"));
        self.context_resolver
            .register_protocol_as_context(verum_common::Text::from("MacroState"));
        self.context_resolver
            .register_protocol_as_context(verum_common::Text::from("StageInfo"));
        self.context_resolver
            .register_protocol_as_context(verum_common::Text::from("Hygiene"));
        self.context_resolver
            .register_protocol_as_context(verum_common::Text::from("CodeSearch"));
        self.context_resolver
            .register_protocol_as_context(verum_common::Text::from("ProjectInfo"));
        self.context_resolver
            .register_protocol_as_context(verum_common::Text::from("SourceMap"));
        self.context_resolver
            .register_protocol_as_context(verum_common::Text::from("Schema"));
        self.context_resolver
            .register_protocol_as_context(verum_common::Text::from("DepGraph"));
        self.context_resolver
            .register_protocol_as_context(verum_common::Text::from("MetaBench"));

        // Define meta context types so they can be referenced
        for ctx_name in &[
            "MetaTypes",
            "MetaRuntime",
            "BuildAssets",
            "CompileDiag",
            "AstAccess",
            "MacroState",
            "StageInfo",
            "Hygiene",
            "CodeSearch",
            "ProjectInfo",
            "SourceMap",
            "Schema",
            "DepGraph",
            "MetaBench",
        ] {
            let ctx_ty = Type::Generic {
                name: verum_common::Text::from(*ctx_name),
                args: List::new(),
            };
            self.ctx
                .define_type(verum_common::Text::from(*ctx_name), ctx_ty.clone());
            self.ctx.env.insert(
                verum_common::Text::from(*ctx_name),
                TypeScheme::mono(ctx_ty),
            );
        }
    }

    /// Register meta built-in function signatures in the type checker environment.
    /// These are functions available inside `meta fn` bodies (compile-time evaluation).
    /// The actual implementations are in verum_compiler/src/meta/builtins/.
    fn register_meta_builtins(&mut self) {
        // Use macros for ergonomic registration
        macro_rules! reg {
            ($name:expr, $params:expr, $ret:expr) => {
                self.ctx
                    .env
                    .insert_mono($name, Type::function($params, $ret));
            };
        }
        // Generic meta builtin: accepts type parameters (e.g., type_name<Int>())
        macro_rules! reg_generic {
            ($name:expr, $params:expr, $ret:expr) => {{
                let tv = TypeVar::fresh();
                let ty = Type::function($params, $ret);
                let scheme = TypeScheme::poly(List::from_iter([tv]), ty);
                self.ctx.env.insert(verum_common::Text::from($name), scheme);
            }};
        }
        // ---- Debugging ----
        reg!("meta_trace_on", List::new(), Type::Unit);
        reg!("meta_trace_off", List::new(), Type::Unit);
        reg!("meta_trace_log", List::from_iter([Type::Text]), Type::Unit);
        reg!("meta_trace_dump", List::new(), Type::Text);
        reg!("meta_trace_lines", List::new(), Type::list(Type::Text));
        reg!("meta_trace_clear", List::new(), Type::Unit);
        reg!("meta_trace_is_enabled", List::new(), Type::Bool);
        reg!("meta_trace_depth", List::new(), Type::Int);
        reg!(
            "meta_trace_enter",
            List::from_iter([Type::Text]),
            Type::Unit
        );
        reg!("meta_trace_exit", List::from_iter([Type::Text]), Type::Unit);

        // ---- Testing (trigger functions) ----
        reg!(
            "trigger_type_reduction_failed",
            List::from_iter([Type::Text, Type::Text]),
            Type::Never
        );
        reg!(
            "trigger_normalization_diverged",
            List::from_iter([Type::Text, Type::Int]),
            Type::Never
        );
        reg!(
            "trigger_smt_verification_failed",
            List::from_iter([Type::Text, Type::Text]),
            Type::Never
        );
        reg!(
            "trigger_proof_construction_failed",
            List::from_iter([Type::Text, Type::Text]),
            Type::Never
        );
        reg!(
            "trigger_refinement_violation",
            List::from_iter([Type::Text, Type::Text]),
            Type::Never
        );
        reg!(
            "trigger_meta_where_unsatisfied",
            List::from_iter([Type::Text]),
            Type::Never
        );

        // ---- Code generation (Tier 0) ----
        reg!("stringify", List::from_iter([Type::Unknown]), Type::Text);
        reg!(
            "concat_idents",
            List::from_iter([Type::Text, Type::Text]),
            Type::Text
        );
        reg!("gensym", List::from_iter([Type::Text]), Type::Text);

        // ---- Reflection (generic + optional regular arg for both call styles) ----
        // These can be called as type_name(Int), type_name<Int>(), etc.
        reg_generic!("type_name", List::from_iter([Type::Unknown]), Type::Text);
        reg_generic!(
            "simple_name_of",
            List::from_iter([Type::Unknown]),
            Type::Text
        );
        reg_generic!("kind_of", List::from_iter([Type::Unknown]), Type::Text);
        reg_generic!("is_struct", List::from_iter([Type::Unknown]), Type::Bool);
        reg_generic!("is_enum", List::from_iter([Type::Unknown]), Type::Bool);
        reg_generic!("is_tuple", List::from_iter([Type::Unknown]), Type::Bool);
        reg_generic!("is_copy", List::from_iter([Type::Unknown]), Type::Bool);
        reg_generic!("is_send", List::from_iter([Type::Unknown]), Type::Bool);
        reg_generic!("is_sync", List::from_iter([Type::Unknown]), Type::Bool);
        reg_generic!("is_sized", List::from_iter([Type::Unknown]), Type::Bool);
        reg_generic!("needs_drop", List::from_iter([Type::Unknown]), Type::Bool);
        reg_generic!(
            "implements",
            List::from_iter([Type::Unknown, Type::Unknown]),
            Type::Bool
        );
        reg_generic!(
            "fields_of",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg_generic!(
            "type_fields",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg_generic!(
            "variants_of",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg_generic!("type_id", List::from_iter([Type::Unknown]), Type::Int);
        reg_generic!("size_of", List::from_iter([Type::Unknown]), Type::Int);
        reg_generic!("align_of", List::from_iter([Type::Unknown]), Type::Int);

        reg_generic!("size_of_val", List::from_iter([Type::Unknown]), Type::Int);
        reg_generic!("min_align_of", List::from_iter([Type::Unknown]), Type::Int);

        // ---- Benchmark / optimization builtins ----
        // black_box prevents the compiler from optimizing away a value
        reg_generic!("black_box", List::from_iter([Type::Unknown]), Type::Unknown);
        // hint::black_box alias
        reg_generic!(
            "hint_black_box",
            List::from_iter([Type::Unknown]),
            Type::Unknown
        );

        // ---- GC builtins ----
        reg!("gc", List::new(), Type::unit());
        reg!("gc_collect", List::new(), Type::unit());
        reg!("gc_stats", List::new(), Type::Unknown);

        // ---- Arithmetic / conversion builtins ----
        reg!("int_to_text", List::from_iter([Type::Int]), Type::Text);
        reg!("text_to_int", List::from_iter([Type::Text]), Type::Int);

        // ---- Runtime ----
        reg!("target_os", List::new(), Type::Text);
        reg!("target_arch", List::new(), Type::Text);
        reg!("is_debug", List::new(), Type::Bool);
        reg!("is_release", List::new(), Type::Bool);
        reg!("has_feature", List::from_iter([Type::Text]), Type::Bool);
        reg!(
            "env",
            List::from_iter([Type::Text]),
            Type::maybe(Type::Text)
        );
        reg!("is_ci", List::new(), Type::Bool);
        reg!("recursion_limit", List::new(), Type::Int);
        reg!("iteration_limit", List::new(), Type::Int);

        // ---- Build assets ----
        reg!("load_text", List::from_iter([Type::Text]), Type::Text);
        reg!("include_str", List::from_iter([Type::Text]), Type::Text);
        reg!("asset_exists", List::from_iter([Type::Text]), Type::Bool);

        // ---- Bitwise operations (meta builtins for @const and meta fn) ----
        reg!(
            "bitwise_and",
            List::from_iter([Type::Int, Type::Int]),
            Type::Int
        );
        reg!(
            "bitwise_or",
            List::from_iter([Type::Int, Type::Int]),
            Type::Int
        );
        reg!(
            "bitwise_xor",
            List::from_iter([Type::Int, Type::Int]),
            Type::Int
        );
        reg!("bitwise_not", List::from_iter([Type::Int]), Type::Int);
        reg!(
            "bitwise_shl",
            List::from_iter([Type::Int, Type::Int]),
            Type::Int
        );
        reg!(
            "bitwise_shr",
            List::from_iter([Type::Int, Type::Int]),
            Type::Int
        );
        // Aliases for bitwise shifts used in some test suites
        reg!(
            "shift_left",
            List::from_iter([Type::Int, Type::Int]),
            Type::Int
        );
        reg!(
            "shift_right",
            List::from_iter([Type::Int, Type::Int]),
            Type::Int
        );
        reg!("text_len", List::from_iter([Type::Text]), Type::Int);

        // ---- Text operations ----
        reg!(
            "text_concat",
            List::from_iter([Type::Text, Type::Text]),
            Type::Text
        );
        reg!(
            "text_split",
            List::from_iter([Type::Text, Type::Text]),
            Type::list(Type::Text)
        );
        reg!(
            "text_join",
            List::from_iter([Type::list(Type::Text), Type::Text]),
            Type::Text
        );
        reg!("text_to_upper", List::from_iter([Type::Text]), Type::Text);
        reg!("text_to_lower", List::from_iter([Type::Text]), Type::Text);
        reg!("text_trim", List::from_iter([Type::Text]), Type::Text);
        reg!(
            "text_replace",
            List::from_iter([Type::Text, Type::Text, Type::Text]),
            Type::Text
        );
        reg!(
            "text_starts_with",
            List::from_iter([Type::Text, Type::Text]),
            Type::Bool
        );
        reg!(
            "text_ends_with",
            List::from_iter([Type::Text, Type::Text]),
            Type::Bool
        );
        reg!(
            "text_contains",
            List::from_iter([Type::Text, Type::Text]),
            Type::Bool
        );
        reg!(
            "text_eq",
            List::from_iter([Type::Text, Type::Text]),
            Type::Bool
        );
        reg!(
            "text_substring",
            List::from_iter([Type::Text, Type::Int, Type::Int]),
            Type::Text
        );
        reg!(
            "text_index_of",
            List::from_iter([Type::Text, Type::Text]),
            Type::Int
        );
        reg!(
            "text_char_at",
            List::from_iter([Type::Text, Type::Int]),
            Type::Char
        );
        reg!(
            "text_repeat",
            List::from_iter([Type::Text, Type::Int]),
            Type::Text
        );
        reg!("text_is_empty", List::from_iter([Type::Text]), Type::Bool);
        reg!(
            "text_lines",
            List::from_iter([Type::Text]),
            Type::list(Type::Text)
        );
        // Aliases
        reg!(
            "char_at",
            List::from_iter([Type::Text, Type::Int]),
            Type::Char
        );
        reg!("text_upper", List::from_iter([Type::Text]), Type::Text);
        reg!("text_lower", List::from_iter([Type::Text]), Type::Text);

        // ---- List operations ----
        reg!(
            "list_len",
            List::from_iter([Type::list(Type::Unknown)]),
            Type::Int
        );
        reg!(
            "list_push",
            List::from_iter([Type::list(Type::Unknown), Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg!(
            "list_get",
            List::from_iter([Type::list(Type::Unknown), Type::Int]),
            Type::Unknown
        );
        reg!(
            "list_map",
            List::from_iter([Type::list(Type::Unknown), Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg!(
            "list_filter",
            List::from_iter([Type::list(Type::Unknown), Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg!(
            "list_fold",
            List::from_iter([Type::list(Type::Unknown), Type::Unknown, Type::Unknown]),
            Type::Unknown
        );
        reg!(
            "list_concat",
            List::from_iter([Type::list(Type::Unknown), Type::list(Type::Unknown)]),
            Type::list(Type::Unknown)
        );
        reg!(
            "list_reverse",
            List::from_iter([Type::list(Type::Unknown)]),
            Type::list(Type::Unknown)
        );
        reg!(
            "list_first",
            List::from_iter([Type::list(Type::Unknown)]),
            Type::Unknown
        );
        reg!(
            "list_last",
            List::from_iter([Type::list(Type::Unknown)]),
            Type::Unknown
        );
        reg!(
            "list_head",
            List::from_iter([Type::list(Type::Unknown)]),
            Type::Unknown
        );
        reg!(
            "list_contains",
            List::from_iter([Type::list(Type::Unknown), Type::Unknown]),
            Type::Bool
        );
        reg!(
            "list_find",
            List::from_iter([Type::list(Type::Unknown), Type::Unknown]),
            Type::Unknown
        );
        reg!(
            "list_index_of",
            List::from_iter([Type::list(Type::Unknown), Type::Unknown]),
            Type::Int
        );
        reg!(
            "list_is_empty",
            List::from_iter([Type::list(Type::Unknown)]),
            Type::Bool
        );
        reg!(
            "list_all",
            List::from_iter([Type::list(Type::Unknown), Type::Unknown]),
            Type::Bool
        );
        reg!(
            "list_any",
            List::from_iter([Type::list(Type::Unknown), Type::Unknown]),
            Type::Bool
        );
        reg!(
            "list_clear",
            List::from_iter([Type::list(Type::Unknown)]),
            Type::list(Type::Unknown)
        );
        reg!(
            "list_tail",
            List::from_iter([Type::list(Type::Unknown)]),
            Type::list(Type::Unknown)
        );
        reg!(
            "list_take",
            List::from_iter([Type::list(Type::Unknown), Type::Int]),
            Type::list(Type::Unknown)
        );
        reg!(
            "list_skip",
            List::from_iter([Type::list(Type::Unknown), Type::Int]),
            Type::list(Type::Unknown)
        );
        reg!(
            "list_zip",
            List::from_iter([Type::list(Type::Unknown), Type::list(Type::Unknown)]),
            Type::list(Type::Unknown)
        );
        reg!(
            "list_flatten",
            List::from_iter([Type::list(Type::Unknown)]),
            Type::list(Type::Unknown)
        );
        reg!(
            "list_sort",
            List::from_iter([Type::list(Type::Unknown)]),
            Type::list(Type::Unknown)
        );
        reg!(
            "list_dedup",
            List::from_iter([Type::list(Type::Unknown)]),
            Type::list(Type::Unknown)
        );

        // ---- Maybe operations ----
        reg!(
            "maybe_unwrap",
            List::from_iter([Type::Unknown]),
            Type::Unknown
        );
        reg!(
            "maybe_unwrap_or",
            List::from_iter([Type::Unknown, Type::Unknown]),
            Type::Unknown
        );
        reg!(
            "maybe_is_some",
            List::from_iter([Type::Unknown]),
            Type::Bool
        );
        reg!(
            "maybe_is_none",
            List::from_iter([Type::Unknown]),
            Type::Bool
        );

        // ---- Map operations ----
        reg!("map_new", List::new(), Type::Unknown);
        reg!("map_len", List::from_iter([Type::Unknown]), Type::Int);
        reg!(
            "map_get",
            List::from_iter([Type::Unknown, Type::Text]),
            Type::Unknown
        );
        reg!(
            "map_insert",
            List::from_iter([Type::Unknown, Type::Text, Type::Unknown]),
            Type::Unknown
        );
        reg!(
            "map_remove",
            List::from_iter([Type::Unknown, Type::Text]),
            Type::Unknown
        );
        reg!(
            "map_contains",
            List::from_iter([Type::Unknown, Type::Text]),
            Type::Bool
        );
        reg!(
            "map_keys",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Text)
        );
        reg!(
            "map_values",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg!(
            "map_entries",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg!("map_is_empty", List::from_iter([Type::Unknown]), Type::Bool);
        reg!("map_clear", List::from_iter([Type::Unknown]), Type::Unknown);
        reg!(
            "map_contains_key",
            List::from_iter([Type::Unknown, Type::Text]),
            Type::Bool
        );

        // ---- Set operations ----
        reg!("set_new", List::new(), Type::Unknown);
        reg!("set_len", List::from_iter([Type::Unknown]), Type::Int);
        reg!(
            "set_insert",
            List::from_iter([Type::Unknown, Type::Unknown]),
            Type::Unknown
        );
        reg!(
            "set_remove",
            List::from_iter([Type::Unknown, Type::Unknown]),
            Type::Unknown
        );
        reg!(
            "set_contains",
            List::from_iter([Type::Unknown, Type::Unknown]),
            Type::Bool
        );
        reg!(
            "set_to_list",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg!(
            "set_union",
            List::from_iter([Type::Unknown, Type::Unknown]),
            Type::Unknown
        );
        reg!(
            "set_intersection",
            List::from_iter([Type::Unknown, Type::Unknown]),
            Type::Unknown
        );
        reg!(
            "set_difference",
            List::from_iter([Type::Unknown, Type::Unknown]),
            Type::Unknown
        );
        reg!("set_is_empty", List::from_iter([Type::Unknown]), Type::Bool);
        reg!("set_clear", List::from_iter([Type::Unknown]), Type::Unknown);

        // ---- Miscellaneous builtins ----
        reg!("bytes_len", List::from_iter([Type::Unknown]), Type::Int);
        reg_generic!("is_generic", List::from_iter([Type::Unknown]), Type::Bool);

        // ---- Arithmetic ----
        reg!("abs", List::from_iter([Type::Int]), Type::Int);
        reg!("min", List::from_iter([Type::Int, Type::Int]), Type::Int);
        reg!("max", List::from_iter([Type::Int, Type::Int]), Type::Int);
        reg!(
            "clamp",
            List::from_iter([Type::Int, Type::Int, Type::Int]),
            Type::Int
        );
        reg!("pow", List::from_iter([Type::Int, Type::Int]), Type::Int);

        // ---- Type properties (generic + optional regular arg) ----
        reg_generic!("stride_of", List::from_iter([Type::Unknown]), Type::Int);
        reg_generic!("type_bits", List::from_iter([Type::Unknown]), Type::Int);
        reg_generic!("type_min", List::from_iter([Type::Unknown]), Type::Int);
        reg_generic!("type_max", List::from_iter([Type::Unknown]), Type::Int);
        reg_generic!("bounds_of", List::from_iter([Type::Unknown]), Type::Unknown);
        reg_generic!("type_of", List::from_iter([Type::Unknown]), Type::Text);
        reg_generic!(
            "element_type_of",
            List::from_iter([Type::Unknown]),
            Type::Text
        );
        reg_generic!(
            "inner_type_of",
            List::from_iter([Type::Unknown]),
            Type::Text
        );
        reg_generic!(
            "key_value_types_of",
            List::from_iter([Type::Unknown]),
            Type::Unknown
        );
        reg_generic!(
            "field_offset",
            List::from_iter([Type::Unknown, Type::Text]),
            Type::Int
        );
        reg_generic!(
            "memory_layout_of",
            List::from_iter([Type::Unknown]),
            Type::Unknown
        );
        reg_generic!("ownership_of", List::from_iter([Type::Unknown]), Type::Text);
        reg_generic!("doc_of", List::from_iter([Type::Unknown]), Type::Text);
        reg_generic!(
            "attributes_of",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg_generic!(
            "has_attribute",
            List::from_iter([Type::Unknown, Type::Text]),
            Type::Bool
        );
        reg_generic!(
            "get_attribute",
            List::from_iter([Type::Unknown, Type::Text]),
            Type::Unknown
        );
        reg_generic!(
            "generics_of",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg_generic!(
            "protocols_of",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg_generic!(
            "super_types_of",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg_generic!(
            "associated_types_of",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg_generic!(
            "lifetime_params_of",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg_generic!(
            "functions_of",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg_generic!(
            "static_functions_of",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg_generic!(
            "instance_methods_of",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg_generic!(
            "where_clause_of",
            List::from_iter([Type::Unknown]),
            Type::list(Type::Unknown)
        );
        reg_generic!("module_of", List::from_iter([Type::Unknown]), Type::Text);

        // ---- Code generation ----
        reg!("quote", List::from_iter([Type::Unknown]), Type::Unknown);
        reg!("unquote", List::from_iter([Type::Unknown]), Type::Unknown);
        reg!("format_ident", List::from_iter([Type::Text]), Type::Text);
        reg!("compile_error", List::from_iter([Type::Text]), Type::Never);
        reg!("compile_warning", List::from_iter([Type::Text]), Type::Unit);
        reg!("ident", List::from_iter([Type::Text]), Type::Unknown);
        reg!("span", List::from_iter([Type::Unknown]), Type::Unknown);

        // ---- Build assets (additional) ----
        reg!(
            "include_bytes",
            List::from_iter([Type::Text]),
            Type::Unknown
        );
        reg!(
            "asset_list_dir",
            List::from_iter([Type::Text]),
            Type::list(Type::Text)
        );
        reg!(
            "asset_metadata",
            List::from_iter([Type::Text]),
            Type::Unknown
        );

        // ---- Runtime (additional) ----
        reg!("target_triple", List::new(), Type::Text);
        reg!("target_pointer_width", List::new(), Type::Int);
        reg!("target_endian", List::new(), Type::Text);
        reg!(
            "target_has_feature",
            List::from_iter([Type::Text]),
            Type::Bool
        );
        reg!("compiler_version", List::new(), Type::Text);
        reg!("opt_level", List::new(), Type::Int);
        reg!("memory_limit", List::new(), Type::Int);
        reg!("timeout_ms", List::new(), Type::Int);
        reg!("module_path", List::new(), Type::Text);
        reg!("crate_name", List::new(), Type::Text);
        reg!("crate_version", List::new(), Type::Text);
        reg!("cog_name", List::new(), Type::Text);
        reg!("cog_version", List::new(), Type::Text);
        reg!("enabled_features", List::new(), Type::list(Type::Text));
        reg!(
            "runtime_config",
            List::from_iter([Type::Text]),
            Type::Unknown
        );
        reg!("config_get", List::from_iter([Type::Text]), Type::Unknown);
        reg!("config_get_int", List::from_iter([Type::Text]), Type::Int);
        reg!("config_get_bool", List::from_iter([Type::Text]), Type::Bool);
        reg!(
            "config_get_array",
            List::from_iter([Type::Text]),
            Type::list(Type::Unknown)
        );

        // ---- Stage information ----
        reg!("stage_current", List::new(), Type::Int);
        reg!("stage_max", List::new(), Type::Int);
        reg!("stage_is_compile_time", List::new(), Type::Bool);
        reg!("stage_is_runtime", List::new(), Type::Bool);
        reg!("stage_is_valid", List::from_iter([Type::Int]), Type::Bool);
        reg!(
            "stage_is_valid_transition",
            List::from_iter([Type::Int, Type::Int]),
            Type::Bool
        );
        reg!("stage_is_enabled", List::from_iter([Type::Int]), Type::Bool);
        reg!(
            "stage_is_max_stage",
            List::from_iter([Type::Int]),
            Type::Bool
        );
        reg!(
            "stage_can_generate",
            List::from_iter([Type::Int, Type::Int]),
            Type::Bool
        );
        reg!(
            "stage_functions_at",
            List::from_iter([Type::Int]),
            Type::list(Type::Unknown)
        );
        reg!(
            "stage_function_stage",
            List::from_iter([Type::Text]),
            Type::Int
        );
        reg!(
            "stage_generation_chain",
            List::from_iter([Type::Int]),
            Type::list(Type::Int)
        );
        reg!("stage_recursion_limit", List::new(), Type::Int);
        reg!("stage_iteration_limit", List::new(), Type::Int);
        reg!("stage_memory_limit", List::new(), Type::Int);
        reg!("stage_quote_depth", List::new(), Type::Int);
        reg!(
            "stage_quote_target",
            List::from_iter([Type::Int]),
            Type::Int
        );
        reg!(
            "stage_trace_marker",
            List::from_iter([Type::Text]),
            Type::Unit
        );
        reg!(
            "stage_unique_ident",
            List::from_iter([Type::Text]),
            Type::Text
        );

        // ---- Debugging (additional) ----
        reg!(
            "meta_trace_value",
            List::from_iter([Type::Text, Type::Unknown]),
            Type::Unknown
        );
        reg!(
            "meta_trace_assert",
            List::from_iter([Type::Bool, Type::Text]),
            Type::Unit
        );

        // ---- Project info ----
        reg!("project_root", List::new(), Type::Text);
        reg!("project_source_dir", List::new(), Type::Text);
        reg!("project_package_name", List::new(), Type::Text);
        reg!("project_package_version", List::new(), Type::Text);
        reg!(
            "project_package_authors",
            List::new(),
            Type::list(Type::Text)
        );
        reg!("project_target_os", List::new(), Type::Text);
        reg!("project_target_arch", List::new(), Type::Text);
        reg!("project_is_debug", List::new(), Type::Bool);
        reg!("project_is_release", List::new(), Type::Bool);
        reg!(
            "project_dependencies",
            List::new(),
            Type::list(Type::Unknown)
        );
        reg!(
            "project_has_dependency",
            List::from_iter([Type::Text]),
            Type::Bool
        );
        reg!(
            "project_enabled_features",
            List::new(),
            Type::list(Type::Text)
        );
        reg!(
            "project_is_feature_enabled",
            List::from_iter([Type::Text]),
            Type::Bool
        );

        // ---- Field access ----
        reg!(
            "field_access",
            List::from_iter([Type::Unknown, Type::Text]),
            Type::Unknown
        );
    }

    /// Set the name resolver (for integration with module loader)
    /// Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Custom name resolution scopes
    pub fn set_name_resolver(&mut self, resolver: NameResolver) {
        self.module_resolver = resolver;
    }

    /// Get mutable reference to the name resolver
    pub fn name_resolver_mut(&mut self) -> &mut NameResolver {
        &mut self.module_resolver
    }

    /// Set the module registry for cross-file type resolution.
    ///

    /// The registry allows the type checker to look up types defined in other modules
    /// when processing imports like `import domain.errors.{RegistryError}`.
    ///

    /// This method takes a locked registry and clones its contents into the type checker's
    /// local registry. This is safe because the registry is read-only during type checking.
    ///

    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — Module-qualified type access
    pub fn set_module_registry(&mut self, registry: Shared<parking_lot::RwLock<ModuleRegistry>>) {
        // Share the SAME handle — both module_registry and
        // session_registry now point at the one authoritative
        // registry owned by the session/pipeline. This used to
        // deep-clone the registry's contents into a second Shared,
        // causing the two copies to drift as lazy-loaded modules
        // were registered into one but not the other.
        self.module_registry = registry.clone();
        self.session_registry = Some(registry);
    }

    /// Set the module registry directly (for testing or when you have
    /// an owned registry).
    pub fn set_module_registry_direct(&mut self, registry: ModuleRegistry) {
        self.module_registry = Shared::new(parking_lot::RwLock::new(registry));
    }

    /// Signal that stdlib registration is complete and user code is being processed.
    /// In user code phase, variant short-name protection is relaxed: user-defined
    /// monomorphic unit variants can shadow polymorphic stdlib unit variants.
    pub fn set_user_code_phase(&mut self) {
        self.user_code_phase = true;
    }

    /// Get the module registry (same handle the session owns).
    pub fn module_registry(&self) -> &Shared<parking_lot::RwLock<ModuleRegistry>> {
        &self.module_registry
    }

    /// Get a clone of the type registry for passing to codegen
    /// This enables type inference information to flow from type checking to code generation.
    /// Types are resolved using the current unifier substitution to ensure concrete types.
    pub fn take_type_registry(&self) -> crate::type_registry::TypeRegistry {
        // Apply unifier substitution to all types in the registry
        // This resolves type variables to their concrete types
        let mut registry = self.type_registry.clone();
        registry.apply_substitution(&self.unifier);
        registry
    }

    /// Set the current module context for type checking
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports
    pub fn set_current_module(&mut self, module_id: crate::context::ModuleId) {
        self.ctx.set_current_module(module_id);
    }

    /// Get the current module context
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports
    pub fn current_module(&self) -> Maybe<crate::context::ModuleId> {
        self.ctx.current_module()
    }

    /// Set the current module path for import resolution
    /// Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Path resolution in imports
    pub fn set_current_module_path(&mut self, path: impl Into<Text>) {
        self.current_module_path = path.into();
    }

    /// MOD-MED-2 — set the user's cog name so
    /// `ImportOrigin::classify` can distinguish project-owned modules
    /// from stdlib/external during glob shadow arbitration. Should be
    /// called once at the start of `phase_type_check` from the manifest.
    pub fn set_current_cog_name(&mut self, name: impl Into<Text>) {
        self.current_cog_name = name.into();
    }

    /// MOD-MED-2 — central glob-shadow arbiter. Decides
    /// whether the incoming glob (provenance `incoming`) is allowed
    /// to register / overwrite the entry currently held under `name`
    /// in the env. Returns `true` to allow registration, `false` to
    /// keep the existing entry (the caller MUST then skip the
    /// overwrite to preserve determinism).
    ///

    /// Side effects: emits `W_STDLIB_SHADOW` diagnostic when a
    /// project glob successfully evicts a stdlib glob — the user
    /// gets a heads-up that their type-decl shadowed a stdlib
    /// constructor with the same short name, which is a common
    /// foot-gun.
    ///

    /// Bookkeeping: on allowed registration the helper updates
    /// `glob_import_provenance[name]` with the incoming provenance.
    pub(crate) fn glob_shadow_arbiter(
        &mut self,
        name: &str,
        incoming: crate::import_origin::ImportProvenance,
    ) -> bool {
        use verum_diagnostics::DiagnosticBuilder;
        let key: Text = verum_common::Text::from(name);
        if let Some(existing) = self.glob_import_provenance.get(&key).cloned() {
            if !crate::import_origin::ImportProvenance::allows_overwrite(&existing, &incoming) {
                // The incoming entry would lose the conflict — keep
                // the existing one. No env mutation, no warning.
                return false;
            }
            // Overwrite is allowed. Emit a stdlib-shadow warning
            // when a project / external glob is evicting a stdlib
            // entry — the user usually wants to know about that.
            if existing.origin == crate::import_origin::ImportOrigin::Stdlib
                && incoming.origin != crate::import_origin::ImportOrigin::Stdlib
            {
                let diag = DiagnosticBuilder::warning()
                    .code("W_STDLIB_SHADOW")
                    .message(format!(
                        "{} mount '{}' shadows stdlib name '{}' (was: {} '{}')",
                        incoming.origin.label(),
                        incoming.module_path.as_str(),
                        name,
                        existing.origin.label(),
                        existing.module_path.as_str(),
                    ))
                    .build();
                self.diagnostics.push(diag);
            }
        }
        self.glob_import_provenance.insert(key, incoming);
        true
    }

    /// Register a type definition in the current module's scope, ALONGSIDE the
    /// unqualified flat registration that feeds `ctx.lookup_type`.
    ///

    /// Motivation (architectural): stdlib module A and stdlib module B can
    /// legitimately both declare `public type RecvError is …`. Prior to this
    /// helper, the flat `ctx.type_defs` map was the only store, and whichever
    /// module loaded last won the unqualified name — which meant signatures
    /// compiled *inside module A* could silently resolve `RecvError` to B's
    /// definition (the last-registered one), producing confusing
    /// cross-module type mismatches at call sites (see
    /// `broadcast_stream.vr` vs `quic.stream_sm.recv`).
    ///

    /// This helper additionally registers the type under the fully qualified
    /// key `{module_path}.{name}`. Resolver code (`ast_to_type_inner` for
    /// `TypeKind::Path`) then tries the qualified key first before falling
    /// back to the unqualified one, so types compiled inside a module always
    /// see that module's own definitions first — regardless of load order.
    ///

    /// No-op fallback (`current_module_path == "cog"` or empty) means
    /// user-code phase / unknown-module context: register only flat, so
    /// behaviour matches pre-change semantics there.
    pub(crate) fn define_type_in_current_module(&mut self, name: Text, ty: Type) {
        let mod_path = self.current_module_path.clone();
        // Always write the unqualified flat entry (back-compat + fast path).
        self.ctx.define_type(name.clone(), ty.clone());
        // Additionally publish under the fully qualified name when we have a
        // real module path to attribute the type to.
        if !mod_path.as_str().is_empty() && mod_path.as_str() != "cog" {
            let qualified: Text = format!("{}.{}", mod_path, name).into();
            self.ctx.define_type(qualified, ty);
        }

        // Architectural: evict stale variant-constructor shadow on this
        // same simple name. A previously-loaded stdlib enum may have
        // registered `Frame: fn(Text) -> PumpError` in `env` because
        // one of its variants happens to be named `Frame`. Now that the
        // actual type `Frame` is being registered, `Frame.MaxData(...)`
        // (the canonical variant-constructor spelling) must see `Frame`
        // as a type — not as the parent enum's variant-ctor function.
        //

        // Only evict when the env binding is a function returning a
        // DIFFERENT Variant/Generic/Named type: that is the signature
        // of a variant-ctor shadow. Plain user functions or functions
        // returning the same-named type are left alone.
        //

        // Qualified bindings (`PumpError.Frame`) stay intact so code
        // that explicitly spells out the parent enum keeps working.
        if let Some(existing) = self.ctx.env.lookup(name.as_str()) {
            let short_name_str = name.as_str();
            let is_variant_ctor_shadow = match &existing.ty {
                Type::Function { return_type, .. } => match return_type.as_ref() {
                    Type::Variant(_) => true,
                    Type::Generic { name: ret_name, .. } => ret_name.as_str() != short_name_str,
                    Type::Named { path, .. } => path.last_segment_name() != short_name_str,
                    _ => false,
                },
                _ => false,
            };
            if is_variant_ctor_shadow {
                let _ = self.ctx.env.remove(short_name_str);
            }
        }
    }

    /// Get the current module path
    /// Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Path resolution in imports
    pub fn current_module_path(&self) -> &Text {
        &self.current_module_path
    }

    /// Set module-level type inference context (COMPLETE implementation)
    ///

    /// This enables full module-level type inference with:
    /// - Cross-function type inference and propagation
    /// - Mutual recursion support via fixpoint iteration
    /// - Polymorphic recursion
    /// - Higher-rank types
    ///

    /// Once set, function type lookups will use the module context
    /// for cross-function inference.
    pub fn set_module_context(&mut self, module_ctx: crate::module_context::ModuleContext) {
        self.module_context = Maybe::Some(module_ctx);
    }

    /// Get the module-level type inference context
    pub fn module_context(&self) -> Maybe<&crate::module_context::ModuleContext> {
        self.module_context.as_ref()
    }

    /// Get mutable reference to module context
    pub fn module_context_mut(&mut self) -> Maybe<&mut crate::module_context::ModuleContext> {
        self.module_context.as_mut()
    }

    /// Register a protocol as a valid context type for use in `using` clauses.
    ///

    /// This is essential for cross-file context resolution where protocols are
    /// defined in one module and used in `using [ProtocolName]` clauses in another.
    ///

    /// # Arguments
    ///

    /// * `name` - The protocol name to register as a valid context
    ///

    /// # Example
    ///

    /// ```ignore
    /// // In module A:
    /// type Database is protocol { ... }
    ///

    /// // In module B:
    /// fn handler() using [Database] { ... } // Database must be registered
    /// ```
    ///

    /// Context type system integration: context requirements tracked in function types, checked at call sites — Cross-file contexts
    pub fn register_protocol_as_context(&mut self, name: Text) {
        self.context_resolver.register_protocol_as_context(name);
    }

    /// Register a stdlib context in BOTH the resolver and the
    /// context checker. This ensures `using [Name]` resolves at
    /// both resolution and type-checking levels — required for
    /// contexts extracted from the embedded stdlib archive where
    /// the declaring module hasn't been type-checked yet.
    pub fn register_stdlib_context(&mut self, name: Text) {
        self.context_resolver
            .register_protocol_as_context(name.clone());
        self.context_checker
            .register_context(name, verum_ast::decl::ContextDecl::synthetic());
    }

    /// Register a stdlib context with full method signatures from
    /// its parsed `ContextDecl` AST node. Replicates the full
    /// registration path from `check_item(ItemKind::Context)`:
    ///  1. Store in context_declarations map
    ///  2. Build Record type from methods
    ///  3. Register type in context resolver
    ///  4. Register in context checker
    /// This enables `ComputeDevice.device_type()` method calls
    /// to type-check correctly even when the declaring module
    /// hasn't been type-checked.
    pub fn register_stdlib_context_full(&mut self, name: Text, decl: verum_ast::decl::ContextDecl) {
        // Step 1: store declaration for method-level lookups
        self.context_declarations.insert(name.clone(), decl.clone());

        // Step 2+3: build context type and register with resolver.
        // If full type building fails (unknown types in method
        // sigs because the declaring module's types aren't
        // registered yet), build a fallback Record with
        // Type::Unknown return types. This enables method
        // resolution to succeed (returning Unknown), which is
        // correct for lenient stdlib mode — the actual types
        // will be checked at VBC codegen time.
        let context_type = match self.build_context_type_from_decl(&decl) {
            Ok(ty) => ty,
            Err(_) => {
                // Fallback: Record with Unknown return types
                let mut fields = indexmap::IndexMap::new();
                for method in &decl.methods {
                    let param_count = method
                        .params
                        .iter()
                        .filter(|p| {
                            !matches!(
                                p.kind,
                                verum_ast::decl::FunctionParamKind::SelfRef
                                    | verum_ast::decl::FunctionParamKind::SelfRefMut
                                    | verum_ast::decl::FunctionParamKind::SelfValue
                                    | verum_ast::decl::FunctionParamKind::SelfValueMut
                            )
                        })
                        .count();
                    let params = (0..param_count).map(|_| Type::Unknown).collect();
                    let method_type = Type::Function {
                        params,
                        return_type: Box::new(Type::Unknown),
                        properties: None,
                        contexts: None,
                        type_params: verum_common::List::new(),
                    };
                    fields.insert(method.name.name.clone(), method_type);
                }
                Type::Record(fields)
            }
        };
        self.context_resolver
            .register_context_type(name.clone(), context_type);

        // Step 3b: register as protocol-as-context for resolver
        self.context_resolver
            .register_protocol_as_context(name.clone());

        // Step 4: register in checker
        self.context_checker.register_context(name.clone(), decl);
    }

    /// Enable/disable lenient context checking. In lenient mode,
    /// undefined-context and missing-method errors are suppressed.
    /// Used temporarily during stdlib context pre-registration.
    pub fn set_lenient_context_checking(&mut self, lenient: bool) {
        self.context_checker.set_lenient(lenient);
    }

    /// Register multiple protocols as valid context types.
    ///

    /// Convenience method for registering protocols from module exports.
    ///

    /// # Arguments
    ///

    /// * `names` - Iterator of protocol names to register
    pub fn register_protocols_as_contexts<I>(&mut self, names: I)
    where
        I: IntoIterator<Item = Text>,
    {
        self.context_resolver.register_protocols_as_contexts(names);
    }

    /// Get access to the context resolver (for diagnostics).
    pub fn context_resolver(&self) -> &crate::context_resolution::ContextResolver {
        &self.context_resolver
    }

    /// Get mutable access to the context resolver.
    pub fn context_resolver_mut(&mut self) -> &mut crate::context_resolution::ContextResolver {
        &mut self.context_resolver
    }

    /// Look up function type from module context (if available)
    ///

    /// This enables cross-function type inference by looking up
    /// inferred types from other functions in the same module.
    ///

    /// Resolution strategy:
    /// 1. Check type environment first (where register_function_signature stores signatures)
    /// 2. Fall back to module context for cross-module lookups
    ///

    /// This enables order-independent function resolution within a file by consulting
    /// the environment where all function signatures are pre-registered.
    fn lookup_function_in_module(&self, name: &str) -> Maybe<TypeScheme> {
        // First check the type environment where register_function_signature() stores signatures
        // This enables order-independent function resolution within a file
        if let Some(scheme) = self.ctx.env.lookup(name) {
            return Maybe::Some(scheme.clone());
        }

        // Fall back to module context for cross-module lookups
        if let Maybe::Some(ref mod_ctx) = self.module_context {
            mod_ctx.get_function_type(name).cloned()
        } else {
            Maybe::None
        }
    }

    /// Define a type in a specific module
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports
    pub fn define_module_type(
        &mut self,
        module_id: crate::context::ModuleId,
        name: impl Into<Text>,
        ty: Type,
    ) {
        self.ctx.define_module_type(module_id, name, ty);
    }

    /// Look up a type in a specific module
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports
    pub fn lookup_module_type(
        &self,
        module_id: crate::context::ModuleId,
        name: &str,
    ) -> Maybe<&Type> {
        self.ctx.lookup_module_type(module_id, name)
    }

    /// Get collected diagnostics
    pub fn diagnostics(&self) -> &List<Diagnostic> {
        &self.diagnostics
    }

    /// Check if a function has already been registered (in both env and function_required_params).
    ///

    /// Used by the compilation pipeline's S1 pass to avoid overwriting pre-registered
    /// function signatures from explicitly imported modules with signatures from
    /// unrelated stdlib modules that happen to have the same function name.
    pub fn is_function_preregistered(&self, name: &str) -> bool {
        let name_text = verum_common::Text::from(name);
        self.function_required_params.contains_key(&name_text)
            && self.ctx.env.lookup(name).is_some()
    }

    /// Clear all collected diagnostics
    pub fn clear_diagnostics(&mut self) {
        self.diagnostics.clear();
    }

    /// Add a diagnostic (warning, note, etc.)
    fn emit_diagnostic(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Convert a Path to a string representation for error messages
    pub(crate) fn path_to_string(&self, path: &verum_ast::Path) -> Text {
        use verum_ast::ty::PathSegment;

        // CRITICAL FIX: Handle Self by resolving to actual type name
        // This enables `Self { x, y }` to work inside implement blocks
        if path.segments.len() == 1 {
            if let PathSegment::SelfValue = &path.segments[0] {
                // Resolve Self to the current self type's name
                if let Maybe::Some(ref self_ty) = self.current_self_type {
                    return self.type_to_name(self_ty);
                }
            }
        }

        let parts: List<&str> = path
            .segments
            .iter()
            .map(|seg| match seg {
                PathSegment::Name(ident) => ident.name.as_str(),
                PathSegment::SelfValue => "Self", // Use capitalized Self as fallback
                PathSegment::Super => "super",
                PathSegment::Cog => "cog",
                PathSegment::Relative => ".",
            })
            .collect();
        parts.join(".")
    }

    /// Get the name of a type (for resolving Self to the actual type name)
    fn type_to_name(&self, ty: &Type) -> Text {
        match ty {
            Type::Named { path, .. } => {
                // Get the last segment of the path as the type name
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.last() {
                    return ident.name.as_str().into();
                }
                format!("{}", ty).into()
            }
            Type::Generic { name, .. } => name.clone(),
            Type::Record(_) => "record".into(),
            // CapabilityRestricted: strip annotation and return base type name
            Type::CapabilityRestricted { base, .. } => self.type_to_name(base),
            // Refined types: unwrap to base type for method lookup
            Type::Refined { base, .. } => self.type_to_name(base),
            _ => format!("{}", ty).into(),
        }
    }

    /// Extract record fields from a type (for struct spread syntax).
    /// Handles Type::Record directly and Type::Named by looking up the struct definition.
    fn extract_record_fields(
        &self,
        ty: &Type,
    ) -> Result<indexmap::IndexMap<verum_common::Text, Type>> {
        self.extract_record_fields_impl(ty, 0)
    }

    /// Inner implementation with depth tracking to prevent infinite recursion
    /// when type aliases form cycles (e.g., type A resolves to Named("A")).
    fn extract_record_fields_impl(
        &self,
        ty: &Type,
        depth: usize,
    ) -> Result<indexmap::IndexMap<verum_common::Text, Type>> {
        const MAX_DEPTH: usize = 100;
        if depth > MAX_DEPTH {
            return Err(TypeError::Other(verum_common::Text::from(format!(
                "type alias resolution depth exceeded when extracting record fields from: {}",
                ty
            ))));
        }

        match ty {
            Type::Record(fields) => Ok(fields.clone()),
            Type::Named { path, .. } => {
                // Get the type name from the path
                let name = self.path_to_string(path);

                // Try to look up struct fields under __struct_fields_Name
                let struct_key = format!("__struct_fields_{}", name);
                if let Option::Some(Type::Record(field_types)) =
                    self.ctx.lookup_type(struct_key.as_str())
                {
                    return Ok(field_types.clone());
                }

                // Fall back to looking up the type directly
                if let Option::Some(resolved) = self.ctx.lookup_type(name.as_str()) {
                    // Guard against self-referential aliases: if the resolved type
                    // is the same Named type, stop to prevent infinite recursion.
                    if let Type::Named { path: rp, .. } = resolved {
                        let rname = self.path_to_string(rp);
                        if rname == name {
                            // Self-referential - stop recursion
                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                "Base expression in record spread must be a record type, found: {}",
                                ty
                            ))));
                        }
                    }
                    match resolved {
                        Type::Record(fields) => return Ok(fields.clone()),
                        // Recursively resolve if it's another Named type
                        Type::Named { .. } => {
                            return self.extract_record_fields_impl(resolved, depth + 1);
                        }
                        _ => {}
                    }
                }

                Err(TypeError::Other(verum_common::Text::from(format!(
                    "Base expression in record spread must be a record type, found: {}",
                    ty
                ))))
            }
            Type::Generic { name, .. } => {
                // Handle generic types
                let struct_key = format!("__struct_fields_{}", name);
                if let Option::Some(Type::Record(field_types)) =
                    self.ctx.lookup_type(struct_key.as_str())
                {
                    return Ok(field_types.clone());
                }
                Err(TypeError::Other(verum_common::Text::from(format!(
                    "Base expression in record spread must be a record type, found: {}",
                    ty
                ))))
            }
            _ => Err(TypeError::Other(verum_common::Text::from(format!(
                "Base expression in record spread must be a record type, found: {}",
                ty
            )))),
        }
    }

    /// Extract the element type from a collection type.
    ///

    /// This is used for domain-based type inference in quantifier bindings.
    /// For `forall x in collection. ...`, we need to infer the type of `x`
    /// from the element type of `collection`.
    ///

    /// Supported collection types:
    /// - List<T> → T
    /// - Set<T> → T
    /// - Range<T> → T
    /// - Array<T, N> → T
    /// - Slice<T> → T
    ///

    /// Returns None if the type is not a recognized collection type.
    ///

    /// Quantifier expressions: "forall x in collection: predicate" and "exists x in collection: predicate" as boolean expressions
    pub(crate) fn element_type_of(&self, ty: &Type) -> Option<Type> {
        match ty {
            // Direct array/slice types have explicit element field
            Type::Array { element, .. } => Some((**element).clone()),
            Type::Slice { element } => Some((**element).clone()),

            // Protocol-based: use IntoIterator resolution
            _ => {
                if let Some(resolution) = self
                    .protocol_checker
                    .read()
                    .resolve_into_iterator_protocol(ty)
                {
                    return Some(resolution.item);
                }
                // Fallback heuristic for generic types not yet registered with IntoIterator
                match ty {
                    Type::Generic { args, .. } if args.len() == 1 => args.first().cloned(),
                    Type::Generic { args, .. } if args.len() >= 2 => args.get(1).cloned(),
                    Type::Named { args, .. } if args.len() == 1 => args.first().cloned(),
                    Type::Named { args, .. } if args.len() >= 2 => args.get(1).cloned(),
                    _ => None,
                }
            }
        }
    }

    /// Infer function type from declaration
    ///

    /// This method infers:
    /// - Parameter types from the function signature
    /// - Return type (wrapped in Future<T> for async functions)
    /// - Computational properties including:
    ///  - Async: from is_async flag
    ///  - Fallible: from throws_clause presence
    ///  - Other properties: inferred from function body
    pub(crate) fn infer_function_type(&mut self, func: &verum_ast::FunctionDecl) -> Result<Type> {
        use verum_ast::decl::FunctionParamKind;
        use verum_ast::ty::GenericParamKind;

        // CRITICAL FIX: Register generic type parameters BEFORE processing parameter/return types
        // This allows HKT parameters like F<_> to be resolved in fn map<F<_>: Functor>(x: F<A>)
        // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types
        self.ctx.enter_scope();

        for generic_param in &func.generics {
            match &generic_param.kind {
                GenericParamKind::Type { name, bounds, .. } => {
                    // Regular type parameter - create fresh type variable
                    let tvar = TypeVar::fresh();
                    let type_var = Type::Var(tvar);
                    let name_text: Text = name.name.clone();
                    self.ctx
                        .env
                        .insert(name.name.clone(), TypeScheme::mono(type_var.clone()));
                    self.ctx.define_type(name_text.clone(), type_var);

                    // Register bounds if present
                    if !bounds.is_empty() {
                        if let Ok(protocol_bounds) =
                            self.convert_type_bounds_to_protocol_bounds(bounds)
                        {
                            // CRITICAL: Register bounds in type_var_bounds for auto-deref
                            self.register_type_var_bounds(tvar, protocol_bounds.clone());

                            let type_param =
                                crate::context::TypeParam::new(name_text.clone(), name.span)
                                    .with_bounds(protocol_bounds);
                            self.ctx.env.add_type_param(type_param);
                        }
                        // Also extract and register direct type bounds (like function types)
                        let type_bounds = self.extract_type_bounds_from_ast(bounds);
                        for bound in type_bounds {
                            self.register_type_var_type_bound(tvar, bound);
                        }
                    }
                }
                GenericParamKind::HigherKinded {
                    name,
                    arity: _,
                    bounds,
                } => {
                    // HKT parameter - use TypeVar so it gets instantiated properly
                    let name_text: Text = name.name.clone();
                    let tvar = TypeVar::fresh();
                    let type_var = Type::Var(tvar);

                    self.ctx
                        .env
                        .insert(name.name.clone(), TypeScheme::mono(type_var.clone()));
                    self.ctx.define_type(name_text.clone(), type_var);

                    // Side table: only register when the HKT carries an
                    // explicit protocol bound (e.g., `F<_>: Functor`). Bare
                    // HKT parameters like `F<_>` (common on bare associated
                    // type declarations) have no dispatchable protocol bound
                    // to find methods through, so registering them would only
                    // risk shadowing a real bounded HKT in an outer scope.
                    if !bounds.is_empty() {
                        self.hkt_type_var_by_name.insert(name_text.clone(), tvar);
                    }

                    // Register bounds if present (e.g., F<_>: Functor)
                    if !bounds.is_empty() {
                        if let Ok(protocol_bounds) =
                            self.convert_type_bounds_to_protocol_bounds(bounds)
                        {
                            // CRITICAL: Register bounds in type_var_bounds for auto-deref
                            self.register_type_var_bounds(tvar, protocol_bounds.clone());

                            let type_param =
                                crate::context::TypeParam::new(name_text.clone(), name.span)
                                    .with_bounds(protocol_bounds);
                            self.ctx.env.add_type_param(type_param);
                        }
                        // Also extract and register direct type bounds
                        let type_bounds = self.extract_type_bounds_from_ast(bounds);
                        for bound in type_bounds {
                            self.register_type_var_type_bound(tvar, bound);
                        }
                    }
                }
                GenericParamKind::KindAnnotated {
                    name,
                    kind: kind_ann,
                    bounds,
                } => {
                    // Kind-annotated HKT parameter: F: Type -> Type
                    // Convert the AST KindAnnotation to the type-checker's Kind, register
                    // the type constructor's kind in kind_inferer, then bind the name as a
                    // fresh type variable so it can be instantiated during inference.
                    let name_text: Text = name.name.clone();
                    let tvar = TypeVar::fresh();
                    let type_var = Type::Var(tvar);

                    self.ctx
                        .env
                        .insert(name.name.clone(), TypeScheme::mono(type_var.clone()));
                    self.ctx.define_type(name_text.clone(), type_var);

                    // Build the kind_inference::Kind from the AST KindAnnotation and
                    // register it so that subsequent applications F<A> are kind-checked.
                    let infer_kind = Self::ast_kind_to_infer_kind(kind_ann);
                    self.kind_inferer
                        .register_type_constructor(name_text.clone(), infer_kind);

                    // Register protocol bounds if present (e.g., F: Type -> Type + Functor)
                    if !bounds.is_empty() {
                        if let Ok(protocol_bounds) =
                            self.convert_type_bounds_to_protocol_bounds(bounds)
                        {
                            self.register_type_var_bounds(tvar, protocol_bounds.clone());

                            let type_param =
                                crate::context::TypeParam::new(name_text.clone(), name.span)
                                    .with_bounds(protocol_bounds);
                            self.ctx.env.add_type_param(type_param);
                        }
                        let type_bounds = self.extract_type_bounds_from_ast(bounds);
                        for bound in type_bounds {
                            self.register_type_var_type_bound(tvar, bound);
                        }
                    }
                }
                _ => {
                    // Other param kinds (Meta, Const, Lifetime) handled as needed
                }
            }
        }

        let param_types: Result<List<_>> = func
            .params
            .iter()
            .filter_map(|p| {
                match &p.kind {
                    FunctionParamKind::Regular { ty, .. } => Some(self.ast_to_type(ty)),
                    _ => None, // Skip self parameters for now
                }
            })
            .collect();

        let return_type = if let Some(ref ret_ty) = func.return_type {
            self.ast_to_type(ret_ty)?
        } else {
            Type::unit()
        };

        // Unified throws + generator + async wrap via the FULL helper.
        // Multi-type throws (`throws(A | B)`) become a `Type::Variant`
        // union that `.map_err` closures can destructure correctly;
        // `is_generator` produces `Generator<Y, Unit>` between the
        // throws and async wraps so `async fn*` decls land as
        // `Future<Generator<Y, Unit>>` matching the function-decl
        // path's wrap order.
        let final_return_type = self.wrap_return_type_for_sig_full(
            return_type,
            &func.throws_clause,
            func.is_async,
            func.is_generator,
        );

        // Infer computational properties from the function declaration
        // This handles throws_clause -> Fallible correlation, async -> Async, and body analysis
        let properties = self.property_inferrer.infer_function_decl(func);

        // Exit the scope we entered for generic parameters
        self.ctx.exit_scope();

        Ok(Type::function_with_properties(
            param_types?,
            final_return_type,
            properties,
        ))
    }

    /// Look up associated item on a type path
    /// Searches protocol implementations for associated types and methods
    pub(crate) fn lookup_associated_item(&self, type_path: &Path, item_name: &str) -> Option<Type> {
        // Convert path to Type
        let ty = Type::Named {
            path: type_path.clone(),
            args: List::new(),
        };

        // Use protocol checker's lookup_protocol_method which searches all implementations
        if let Ok(Maybe::Some(method_ty)) = self
            .protocol_checker
            .read()
            .lookup_protocol_method(&ty, &verum_common::Text::from(item_name))
        {
            return Some(method_ty);
        }

        None
    }

    /// Look up item in impl block
    /// Searches both inherent impls and protocol impls for methods/associated items
    pub(crate) fn lookup_impl_item(
        &self,
        ty: Type,
        item_name: &str,
        _span: Span,
    ) -> Result<Option<Type>> {
        // Use protocol checker's lookup_protocol_method which searches all implementations
        match self
            .protocol_checker
            .read()
            .lookup_protocol_method(&ty, &verum_common::Text::from(item_name))
        {
            Ok(Maybe::Some(method_ty)) => Ok(Some(method_ty)),
            Ok(Maybe::None) => Ok(None),
            Err(_) => Ok(None), // Treat protocol errors as "not found"
        }
    }

    /// Generate path suggestions for error messages
    pub(crate) fn generate_path_suggestions(
        &self,
        segments: &List<verum_common::Text>,
    ) -> List<verum_common::Text> {
        let mut suggestions = List::new();

        // Check if first segment might be a known module
        if let Some(first) = segments.first() {
            let first_str = first.as_str();

            // Check common module names
            let common_modules = ["std", "core", "collections", "io", "fs", "net"];
            for module in common_modules {
                if levenshtein_distance(first_str, module) <= 2 {
                    suggestions.push(verum_common::Text::from(module));
                }
            }
        }

        suggestions
    }

    /// Main entry point: infer type for an expression.
    pub fn infer(&mut self, expr: &Expr, mode: InferMode) -> Result<InferResult> {
        let start = Instant::now();
        let result = self.infer_expr(expr, mode)?;
        self.metrics.time_us += start.elapsed().as_micros() as u64;
        Ok(result)
    }

    /// Check an expression against an expected type.
    pub fn check(&mut self, expr: &Expr, expected: Type) -> Result<InferResult> {
        let start = Instant::now();
        self.metrics.check_count += 1;

        let result = self.check_expr(expr, &expected)?;

        self.metrics.time_us += start.elapsed().as_micros() as u64;
        Ok(result)
    }

    /// Type check in synthesis mode.
    // Expression inference methods (synth_expr, check_expr, infer_expr*, infer_block*)
    // → see infer/expr.rs in this module

    /// Type check a statement.
    /// Returns whether the statement diverges (has type Never).
    pub fn check_stmt(&mut self, stmt: &Stmt) -> Result<bool> {
        let _depth_guard = self.inc_inference_depth("check_stmt")?;
        self.check_stmt_inner(stmt)
    }

    /// Inner implementation of check_stmt
    fn check_stmt_inner(&mut self, stmt: &Stmt) -> Result<bool> {
        match &stmt.kind {
            StmtKind::Let { pattern, ty, value } => {
                #[cfg(debug_assertions)]
                {
                    // #[cfg(debug_assertions)]
                    // eprintln!("[DEBUG] check_stmt_inner: StmtKind::Let at {:?}", stmt.span);
                    if let Some(ty_ast) = ty {
                        // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG] type annotation present: {:?}", ty_ast);
                    }
                    if let Some(val) = value {
                        // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG] value expr kind: {:?}", std::mem::discriminant(&val.kind));
                    }
                }
                if let Some(val) = value {
                    #[cfg(debug_assertions)]
                    {
                        // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG] Let: has value, about to check");
                        use std::io::Write;
                        let _ = std::io::stderr().flush();
                    }
                    // ========================================================================
                    // CRITICAL FIX: Use bidirectional type checking for let statements
                    // ========================================================================
                    // When a type annotation is present, use check_expr to propagate the
                    // expected type into the expression. This enables proper type inference
                    // for generic constructors like List.new() and Map.new().
                    //

                    // Example: `let data: List<verum_common::Text> = List.new();`
                    // With check_expr, List.new() knows to instantiate as List<verum_common::Text>.
                    //

                    // Bidirectional type checking: synthesize types bottom-up from expressions, check top-down from annotations
                    // ========================================================================
                    #[cfg(debug_assertions)]
                    {
                        // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG] Let: checking if ty_ast present, ty.is_some()={}", ty.is_some());
                        use std::io::Write;
                        let _ = std::io::stderr().flush();
                    }
                    let actual_ty = if let Some(ty_ast) = ty {
                        #[cfg(debug_assertions)]
                        {
                            // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG] Let: calling ast_to_type");
                            use std::io::Write;
                            let _ = std::io::stderr().flush();
                        }
                        let expected = self.ast_to_type(ty_ast)?;
                        #[cfg(debug_assertions)]
                        {
                            // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG] Let: ast_to_type returned, calling check_expr");
                            use std::io::Write;
                            let _ = std::io::stderr().flush();
                        }
                        // Use check_expr to propagate expected type into the expression
                        let value_result = self.check_expr(val, &expected)?;
                        let value_ty = value_result.ty;
                        // Normalize and unify to ensure types match
                        let normalized_value = self.normalize_type(&value_ty);
                        let normalized_expected = self.normalize_type(&expected);
                        self.unifier
                            .unify(&normalized_value, &normalized_expected, val.span)?;

                        // =====================================================================
                        // REFINEMENT CHECK: Verify value satisfies refinement predicate
                        // Spec: L1-core/dependent/003_sigma_fail — sigma constraint violations
                        // =====================================================================
                        // When the expected type is a refinement type (e.g., Natural = Int{>= 0}),
                        // check that the value expression satisfies the predicate.
                        // This catches violations like `let x: Natural = -5` at compile time.
                        {
                            let check_ty = self.normalize_type(&expected);
                            if let Type::Refined {
                                ref base,
                                ref predicate,
                            } = check_ty
                            {
                                let refinement_type = crate::refinement::RefinementType {
                                    base_type: (**base).clone(),
                                    predicate: predicate.clone(),
                                    span: val.span,
                                };
                                match self.check_refinement_with_evidence(val, &refinement_type) {
                                    Ok(crate::refinement::VerificationResult::Valid) => {
                                        // Predicate satisfied — continue
                                    }
                                    Ok(crate::refinement::VerificationResult::Invalid {
                                        ..
                                    }) => {
                                        // Only report error if syntactic evaluator confirms
                                        if let verum_common::Maybe::Some(
                                            crate::refinement::VerificationResult::Invalid {
                                                ..
                                            },
                                        ) = self.refinement.syntactic_check_only(val, predicate)
                                        {
                                            let pred_text = format!("{}", predicate);
                                            return Err(TypeError::RefinementFailed {
                                                predicate: verum_common::Text::from(pred_text),
                                                span: val.span,
                                            });
                                        }
                                    }
                                    // Unknown or Err — gradual verification
                                    _ => {}
                                }
                            }
                        }

                        expected
                    } else {
                        // No type annotation - synthesize the type
                        #[cfg(debug_assertions)]
                        {
                            // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG] Let: no ty, calling synth_expr");
                            use std::io::Write;
                            let _ = std::io::stderr().flush();
                        }
                        let value_result = self.synth_expr(val)?;
                        value_result.ty
                    };

                    // CRITICAL: Apply unifier to resolve type variables before generalizing
                    // Without this, type variables like τ39 (unified with Handle) would be
                    // quantified over, causing instantiate() to create new fresh variables
                    // that lose the unification information.
                    let resolved_ty = self.unifier.apply(&actual_ty);

                    // =====================================================================
                    // E320: Check stack allocation size
                    // Spec: L0-critical/memory-safety/buffer_overflow/no_stack_overflow.vr
                    // =====================================================================
                    // Prevent stack overflow by rejecting allocations that exceed safe limit.
                    // Large allocations should use Heap<T> instead.
                    self.check_stack_allocation_size(&resolved_ty, stmt.span)?;

                    // =========================================================================
                    // HOF TYPE INFERENCE FIX: Keep closures with unresolved type vars monomorphic
                    // =========================================================================
                    // For higher-order functions to work without explicit type annotations,
                    // closures with unresolved type variables should NOT be generalized.
                    // If we generalize, instantiate() creates NEW type variables at call sites,
                    // and the original type variables in the type registry never get resolved.
                    //

                    // Example: let apply = |f, x| f(x);
                    // - Closure has type fn(T1, T2) -> T3 with fresh TypeVars
                    // - If we generalize: ∀T1,T2,T3. fn(T1,T2) -> T3
                    // - At apply(double, 21): instantiate creates fn(T100,T101) -> T102
                    // - Unification updates T100,T101,T102 but NOT T1,T2,T3
                    // - Type registry entry (keyed by closure span) still has T1,T2,T3
                    //

                    // By keeping the closure monomorphic, the SAME type variables (T1,T2,T3)
                    // are used at both definition and call sites, so unification works.
                    // =========================================================================
                    let is_closure_with_unresolved_vars =
                        matches!(&val.kind, ExprKind::Closure { .. })
                            && !resolved_ty.free_vars().is_empty();

                    // CRITICAL FIX: Mutable bindings with unresolved type vars must stay
                    // monomorphic. Otherwise `let mut m = Map.new()` creates a polymorphic
                    // type forall K V. Map<K,V>, and each usage gets fresh type vars.
                    // Then `m.insert("a", 1)` unifies fresh vars but `m.get("a")` creates
                    // new fresh vars that are NOT unified — returns `_` instead of `Int`.
                    // Same principle as the closure fix: same type vars at all use sites.
                    let is_mutable_binding = matches!(
                        &pattern.kind,
                        verum_ast::PatternKind::Ident { mutable: true, .. }
                    );
                    let is_mutable_with_unresolved_vars =
                        is_mutable_binding && !resolved_ty.free_vars().is_empty();

                    let scheme =
                        if is_closure_with_unresolved_vars || is_mutable_with_unresolved_vars {
                            // Keep monomorphic so unification at call sites updates the
                            // same type variables that are in the type registry
                            TypeScheme::mono(resolved_ty)
                        } else {
                            // Normal let-polymorphism for non-closures or fully resolved closures
                            self.ctx.env.generalize(resolved_ty)
                        };

                    // Bind pattern
                    self.bind_pattern_scheme(pattern, scheme.clone())?;

                    // MLS Phase 2b-Followup propagation (#292):
                    // when binding `let x = expr;`, compute the
                    // expression's classification and seed the
                    // sidecar so `x`'s classification reflects its
                    // source. Only fires for Ident patterns at the
                    // top level — destructuring patterns are
                    // #292-Patterns scope (each sub-binding tracks
                    // independently).
                    //

                    // The lattice's `Public` identity means
                    // unclassified expressions don't pollute the
                    // sparse map: `let y = 42;` doesn't create a
                    // sidecar entry, only `let secret = secret_arg;`
                    // does.
                    if let verum_ast::pattern::PatternKind::Ident { name, .. } = &pattern.kind {
                        let level = self.expr_classification(val);
                        if level != verum_common::mls::MlsLevel::Public {
                            self.classification_map.insert(name.name.clone(), level);
                        }
                    }

                    // =====================================================================
                    // NLL: Link reference holders to their borrows
                    // Spec: L0-critical/reference_system/access_rules/ref_scope_valid
                    // =====================================================================
                    // If the value is a reference expression and we just created a borrow,
                    // link the bound variable to that borrow for NLL tracking.
                    // This enables `*ref_val` to release the borrow of the referenced variable.
                    if let verum_ast::pattern::PatternKind::Ident { name, mutable, .. } =
                        &pattern.kind
                    {
                        // Check if we have a pending borrow to link
                        if self.borrow_tracker.has_last_borrow() {
                            self.borrow_tracker
                                .link_holder_to_last_borrow(name.name.as_str());
                        }
                        // Track mutable bindings for static bounds check bypass
                        if *mutable {
                            self.mutable_bindings.insert(name.name.to_string());
                        }
                    }

                    // =====================================================================
                    // DEFINITE ASSIGNMENT: Mark as fully initialized when value is provided
                    // Spec: L0-critical/memory-safety/uninitialized
                    // =====================================================================
                    if let verum_ast::pattern::PatternKind::Ident { name, .. } = &pattern.kind {
                        self.init_tracker.register_initialized(name.name.as_str());
                    }
                } else {
                    // Let binding without value - need type annotation
                    // =====================================================================
                    // DEFINITE ASSIGNMENT: Track uninitialized compound types
                    // Spec: L0-critical/memory-safety/uninitialized
                    // =====================================================================
                    if let Some(ty_ast) = ty {
                        let ty = self.ast_to_type(ty_ast)?;
                        self.bind_pattern(pattern, &ty)?;

                        // Register uninitialized variable with appropriate tracking
                        if let verum_ast::pattern::PatternKind::Ident { name, .. } = &pattern.kind {
                            let var_name = name.name.as_str();
                            self.register_uninitialized_var(var_name, &ty);
                        }
                    } else {
                        return Err(TypeError::Other(
                            "Let binding without value must have type annotation".to_text(),
                        ));
                    }
                }

                Ok(false)
            }

            StmtKind::Expr { expr, .. } => {
                let result = self.synth_expr(expr)?;
                // Check if the expression diverges (has type Never)
                Ok(matches!(result.ty, Type::Never))
            }

            // Let-else: let pattern = value else { diverging_block }
            // The pattern bindings are available after the let-else
            StmtKind::LetElse {
                pattern,
                ty,
                value,
                else_block,
            } => {
                // CRITICAL FIX: Use bidirectional type checking for let-else statements
                // Same as for regular let statements - use check_expr when annotation present
                let actual_ty = if let Some(ty_ast) = ty {
                    let expected = self.ast_to_type(ty_ast)?;
                    // Use check_expr to propagate expected type into the expression
                    let value_result = self.check_expr(value, &expected)?;
                    let value_ty = value_result.ty;
                    // Normalize types to resolve type aliases before unification
                    let normalized_value = self.normalize_type(&value_ty);
                    let normalized_expected = self.normalize_type(&expected);
                    self.unifier
                        .unify(&normalized_value, &normalized_expected, value.span)?;
                    expected
                } else {
                    // No type annotation - synthesize the type
                    let value_result = self.synth_expr(value)?;
                    value_result.ty
                };

                // Bind pattern variables with the value's type
                // These bindings are visible after the let-else statement
                self.bind_pattern(pattern, &actual_ty)?;

                // Type check the else block (should diverge - return Never type)
                self.check_block(else_block, &Type::never())?;

                Ok(false)
            }

            // Defer statement - type checks the deferred expression
            StmtKind::Defer(expr) => {
                self.synth_expr(expr)?;
                Ok(false)
            }

            // Errdefer statement - type checks the deferred expression (error-path only)
            // Spec: grammar/verum.ebnf v2.8 - Section 2.13 defer_stmt
            StmtKind::Errdefer(expr) => {
                self.synth_expr(expr)?;
                Ok(false)
            }

            // Provide statement - type checks the context value
            // Also validates for duplicate provide statements in same scope (E808)
            StmtKind::Provide { context, value, .. } => {
                // Type check the value expression
                let _value_ty = self.synth_expr(value)?;

                // Register the context as provided and check for duplicates
                // E808: Duplicate provide for same context in same scope
                self.context_checker
                    .provide_context(context.as_str(), stmt.span)?;

                // Make the context name available in the type environment
                // so that code after the provide statement can access context methods.
                // This enables the pattern: provide Logger = impl; Logger.log("msg");
                // Context provision: "provide ContextName = implementation" installs a provider
                // in lexical scope via task-local storage (theta) — Scope Binding
                let context_text: Text = context.clone();
                if let Maybe::Some(ctx_ty) = self.context_resolver.get_context_type(&context_text) {
                    let ctx_ty = ctx_ty.clone();
                    self.ctx
                        .env
                        .insert(context.as_str(), TypeScheme::mono(ctx_ty));
                }

                Ok(false)
            }

            // Item declarations (including nested functions)
            // These must be type-checked and added to the current scope
            StmtKind::Item(item) => {
                // For local implement blocks, we need to register method signatures
                // BEFORE type-checking the block. This is normally done in a separate
                // pass for top-level items, but local items need inline registration.
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG] check_stmt_inner: StmtKind::Item, kind={:?}", std::mem::discriminant(&item.kind));
                if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                    self.register_impl_block(impl_decl)?;
                }
                self.check_item(item)?;
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG] check_stmt_inner: StmtKind::Item completed");
                Ok(false)
            }

            // Block-scoped provide: provide Context = value in { block }
            // The context is pushed before the block and popped after it exits.
            // This implements stack-based context scope management.
            StmtKind::ProvideScope {
                context,
                value,
                block,
                alias: _,
            } => {
                // Type check the context value
                let _value_ty = self.synth_expr(value)?;

                // Enter a new scope for the context and provide it
                self.context_checker.enter_scope();
                // This checks for duplicates in the new scope (won't conflict with outer)
                // so we ignore the result - duplicates in the same ProvideScope aren't possible
                let _ = self
                    .context_checker
                    .provide_context(context.as_str(), stmt.span);

                // Make the context name available within the block scope
                // so that context method calls resolve correctly.
                // Context provision: "provide ContextName = implementation" installs a provider
                // in lexical scope via task-local storage (theta) — Scoped Binding
                let context_text: Text = context.clone();
                if let Maybe::Some(ctx_ty) = self.context_resolver.get_context_type(&context_text) {
                    let ctx_ty = ctx_ty.clone();
                    self.ctx
                        .env
                        .insert(context.as_str(), TypeScheme::mono(ctx_ty));
                }

                // Type check the block expression
                let _block_result = self.synth_expr(block)?;

                // Exit the scope - context is no longer available
                self.context_checker.exit_scope();

                // ProvideScope returns the result of the block, so it may diverge
                // if the block diverges
                Ok(false)
            }

            // Empty statement
            StmtKind::Empty => Ok(false),
        }
    }

    /// Infer the type of a recovery body (either match arms or closure).
    /// Used for try-recover and try-recover-finally expressions.
    ///

    /// `error_type` is the type of the error being recovered from (the `E` in `Result<T, E>`).
    fn infer_recover_body(
        &mut self,
        recover: &verum_ast::expr::RecoverBody,
        error_type: &Type,
    ) -> Result<Type> {
        use verum_ast::expr::RecoverBody;

        match recover {
            RecoverBody::MatchArms { arms, .. } => {
                // Infer type from the first arm, then check all arms unify
                if arms.is_empty() {
                    return Ok(Type::unit());
                }

                // Process first arm with pattern binding
                self.ctx.enter_scope();
                self.bind_pattern(&arms[0].pattern, error_type)?;
                let first_result = self.synth_expr(&arms[0].body)?;
                let result_ty = first_result.ty.clone();
                self.ctx.exit_scope();

                // Check all other arms have the same type
                for arm in arms.iter().skip(1) {
                    self.ctx.enter_scope();
                    self.bind_pattern(&arm.pattern, error_type)?;
                    let arm_result = self.synth_expr(&arm.body)?;
                    self.ctx.exit_scope();
                    self.unifier.unify(&result_ty, &arm_result.ty, arm.span)?;
                }

                Ok(result_ty)
            }
            RecoverBody::Closure { param, body, .. } => {
                // Enter scope for closure parameter
                self.ctx.enter_scope();

                // Bind closure parameter to error type
                // Grammar: recover_closure = closure_params , recover_closure_body
                // The closure takes the error as its parameter
                self.bind_pattern(&param.pattern, error_type)?;

                let result = self.synth_expr(body)?;
                self.ctx.exit_scope();

                Ok(result.ty)
            }
        }
    }

    /// Extract the error type from a try block by analyzing ? operators.
    ///

    /// Extract the error type from the enclosing function's return type.
    /// If the function returns Result<T, E>, returns Some(E).
    /// Returns None if the function doesn't return a Result type.
    fn extract_function_error_type(&self) -> Option<Type> {
        if let Maybe::Some(ref ret_ty) = self.current_function_return_type {
            let resolved = self.unifier.apply(ret_ty);
            // Structural check: Result<T, E> (nominal) or variant { Ok(T), Err(E) }
            if let Some((_, err_ty)) = resolved.as_result() {
                return Some(err_ty);
            }
        }
        None
    }

    /// Scans the try block expression for Try (?) operators and extracts
    /// the error type from the first Result or Maybe type found.
    /// Returns a fresh type variable if no ? operators are found.
    fn extract_try_block_error_type(&mut self, try_block: &Expr) -> Result<Type> {
        // Find the first ? operator in the expression and extract its error type
        if let Some(error_ty) = self.find_try_operator_error_type(try_block) {
            Ok(error_ty)
        } else {
            // No ? operators found - use a fresh type variable
            // This allows the recover block patterns to constrain the type
            Ok(Type::Var(TypeVar::fresh()))
        }
    }

    /// Extract success and error types from a Try-compatible type using PROTOCOL resolution.
    /// This is stdlib-agnostic: uses the Try protocol's Output and Residual associated types.
    /// Returns (success_type, error_type) where Never indicates unknown.
    ///

    /// Design: Uses protocol-based resolution exclusively - no hardcoded type names.
    /// The Try protocol defines: type Output; type Residual;
    fn extract_try_output_types(&self, ty: &Type) -> (Type, Type) {
        // First resolve any type variables
        let resolved = self.unifier.apply(ty);

        // Use protocol-based resolution via the Try protocol
        if let Some(resolution) = self.protocol_checker.read().resolve_try_protocol(&resolved) {
            // Extract error type from residual if possible
            let error_ty = self
                .protocol_checker
                .read()
                .extract_error_from_residual(&resolution.residual)
                .unwrap_or(Type::Never);
            return (resolution.output, error_ty);
        }

        // Type variable: create fresh variables for both
        if let Type::Var(_) = &resolved {
            return (Type::Var(TypeVar::fresh()), Type::Var(TypeVar::fresh()));
        }

        // No Try implementation found
        (Type::Never, Type::Never)
    }

    /// Check if a type is Try-compatible using PROTOCOL resolution.
    /// A type is Try-compatible if it implements the Try protocol.
    /// This is stdlib-agnostic: uses protocol implementation lookup.
    fn is_try_compatible_type(&self, ty: &Type) -> bool {
        let resolved = self.unifier.apply(ty);

        // Type variables are treated as potentially Try-compatible
        if let Type::Var(_) = &resolved {
            return true;
        }

        // Check if the type implements the Try protocol
        self.protocol_checker
            .read()
            .resolve_try_protocol(&resolved)
            .is_some()
    }

    /// Check if a type name corresponds to a heap-allocated type.
    ///

    /// Wrap a value type in a success variant that matches the expected type's structure.
    /// Uses the expected type's variant structure to construct the appropriate wrapper.
    /// This preserves the structural form of the expected type.
    fn wrap_in_success_type(&self, value_ty: &Type, expected: &Type, error_ty: Type) -> Type {
        self.wrap_in_success_type_impl(value_ty, expected, error_ty, 0)
    }

    fn wrap_in_success_type_impl(
        &self,
        value_ty: &Type,
        expected: &Type,
        error_ty: Type,
        depth: usize,
    ) -> Type {
        const MAX_DEPTH: usize = 20;
        if depth > MAX_DEPTH {
            return value_ty.clone();
        }

        let resolved_expected = self.unifier.apply(expected);

        // For Variant types, copy their structure with the new success value
        if let Type::Variant(variants) = &resolved_expected {
            // Get the "success" variant key (first non-Unit payload, or first key)
            // This is structural: we use whatever variant structure the expected type has
            let success_key = variants
                .iter()
                .find(|(_, payload)| **payload != Type::Unit)
                .map(|(key, _)| key.clone())
                .or_else(|| variants.keys().next().cloned());

            if let Some(success_key) = success_key {
                // Build a new Variant with the same structure
                let mut new_variants = indexmap::IndexMap::new();
                for (key, payload) in variants.iter() {
                    if key == &success_key {
                        // This is the success variant - use the value type
                        new_variants.insert(key.clone(), value_ty.clone());
                    } else if *payload == Type::Unit {
                        // Unit payloads stay as Unit
                        new_variants.insert(key.clone(), Type::Unit);
                    } else {
                        // Non-success, non-unit payloads use the error type
                        new_variants.insert(key.clone(), error_ty.clone());
                    }
                }
                return Type::Variant(new_variants);
            }
        }

        // For Generic/Named types, try to resolve and wrap
        if let Type::Generic { .. } | Type::Named { .. } = &resolved_expected {
            if let Some(expanded) = self.expand_type_alias(&resolved_expected) {
                return self.wrap_in_success_type_impl(value_ty, &expanded, error_ty, depth + 1);
            }
        }

        // Fallback: try to use the Try protocol resolution on the expected type
        // to get its structure, then construct accordingly
        if let Some(resolution) = self
            .protocol_checker
            .read()
            .resolve_try_protocol(&resolved_expected)
        {
            // Use the residual structure to construct the result
            // The success type should be value_ty, error from residual
            let mut variants = indexmap::IndexMap::new();
            // Use generic variant names based on protocol resolution
            variants.insert(Text::from("success"), value_ty.clone());
            variants.insert(Text::from("failure"), error_ty);
            return Type::Variant(variants);
        }

        // Ultimate fallback: just return the value type
        value_ty.clone()
    }

    /// Try to expand a type alias to its underlying definition.
    /// Returns None if the type cannot be expanded (not an alias or definition not found).
    /// Uses cycle detection to prevent infinite expansion of self-referential type aliases.
    fn expand_type_alias(&self, ty: &Type) -> Option<Type> {
        match ty {
            Type::Generic { name, args } => {
                // Cycle detection: if we're already expanding this type, stop
                let _cycle_guard = TypeResolutionCycleGuard::try_enter(name.to_string())?;
                // First try the alias table (populated by `type X = Y;` declarations)
                if let Some(alias_target) = self.ctx.resolve_alias(name.as_str()) {
                    if args.is_empty() {
                        return Some(alias_target.clone());
                    }
                    return Some(self.substitute_type_args(alias_target, args));
                }
                // Fallback: look up the type definition table directly by name
                if let Some(def) = self.ctx.lookup_type(name.as_str()) {
                    // Only expand if the definition is a Record or Variant (structural type)
                    if matches!(def, Type::Record(_) | Type::Variant(_)) {
                        if args.is_empty() {
                            return Some(def.clone());
                        }
                        return Some(self.substitute_with_params(name.as_str(), def, args));
                    }
                }
                None
            }
            Type::Named { path, args } => {
                // Extract the type name from the path
                let type_name = path.segments.last().and_then(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str()),
                    _ => None,
                })?;

                // Cycle detection: if we're already expanding this type, stop
                let _cycle_guard = TypeResolutionCycleGuard::try_enter(type_name.to_string())?;

                // First try the alias table
                if let Some(alias_target) = self.ctx.resolve_alias(type_name) {
                    if args.is_empty() {
                        return Some(alias_target.clone());
                    }
                    return Some(self.substitute_type_args(alias_target, args));
                }
                // Fallback: look up the type definition table directly by name
                if let Some(def) = self.ctx.lookup_type(type_name) {
                    if matches!(def, Type::Record(_) | Type::Variant(_)) {
                        if args.is_empty() {
                            return Some(def.clone());
                        }
                        return Some(self.substitute_with_params(type_name, def, args));
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Substitute type arguments into a type definition using the __type_params_ registry.
    /// This handles multi-character parameter names (In, Out, etc.) that the heuristic
    /// substitute_type_args cannot resolve.
    fn substitute_with_params(&self, type_name: &str, def: &Type, args: &[Type]) -> Type {
        // First try to look up the type parameter record to build a name->index mapping
        let params_key = format!("__type_params_{}", type_name);
        if let Some(Type::Record(param_record)) = self.ctx.lookup_type(&params_key) {
            // The param_record maps parameter names to their types (usually Named types)
            // We use the order of keys to determine parameter positions
            let param_names: Vec<&verum_common::Text> = param_record.keys().collect();
            if !param_names.is_empty() && param_names.len() >= args.len() {
                // Build a substitution: replace each Named(param_name) with the corresponding arg
                let substituted = self.substitute_by_param_names(def, &param_names, args);
                return substituted;
            }
        }
        // Fall back to heuristic substitution
        self.substitute_type_args(def, args)
    }

    /// Substitute Named type parameters by matching parameter names to argument positions.
    fn substitute_by_param_names(
        &self,
        ty: &Type,
        param_names: &[&verum_common::Text],
        args: &[Type],
    ) -> Type {
        match ty {
            Type::Named {
                path,
                args: named_args,
            } if named_args.is_empty() => {
                if let Some(ident) = path.as_ident() {
                    let name = ident.name.as_str();
                    for (i, pname) in param_names.iter().enumerate() {
                        if pname.as_str() == name {
                            if let Some(replacement) = args.get(i) {
                                return replacement.clone();
                            }
                        }
                    }
                }
                ty.clone()
            }
            Type::Named {
                path,
                args: named_args,
            } => {
                let new_args: List<Type> = named_args
                    .iter()
                    .map(|a| self.substitute_by_param_names(a, param_names, args))
                    .collect();
                Type::Named {
                    path: path.clone(),
                    args: new_args,
                }
            }
            Type::Generic {
                name,
                args: generic_args,
            } => {
                let new_args: List<Type> = generic_args
                    .iter()
                    .map(|a| self.substitute_by_param_names(a, param_names, args))
                    .collect();
                Type::Generic {
                    name: name.clone(),
                    args: new_args,
                }
            }
            Type::Record(fields) => {
                let mut new_fields = indexmap::IndexMap::new();
                for (key, val) in fields {
                    new_fields.insert(
                        key.clone(),
                        self.substitute_by_param_names(val, param_names, args),
                    );
                }
                Type::Record(new_fields)
            }
            Type::Variant(variants) => {
                let mut new_variants = indexmap::IndexMap::new();
                for (key, val) in variants {
                    new_variants.insert(
                        key.clone(),
                        self.substitute_by_param_names(val, param_names, args),
                    );
                }
                Type::Variant(new_variants)
            }
            Type::Function {
                params,
                return_type,
                contexts,
                properties,
                type_params,
            } => {
                let new_params: List<Type> = params
                    .iter()
                    .map(|p| self.substitute_by_param_names(p, param_names, args))
                    .collect();
                let new_return = self.substitute_by_param_names(return_type, param_names, args);
                Type::Function {
                    params: new_params,
                    return_type: Box::new(new_return),
                    contexts: contexts.clone(),
                    properties: properties.clone(),
                    type_params: type_params.clone(),
                }
            }
            Type::Tuple(elements) => {
                let new_elements: List<Type> = elements
                    .iter()
                    .map(|e| self.substitute_by_param_names(e, param_names, args))
                    .collect();
                Type::Tuple(new_elements)
            }
            Type::Reference { inner, mutable } => Type::Reference {
                inner: Box::new(self.substitute_by_param_names(inner, param_names, args)),
                mutable: *mutable,
            },
            _ => ty.clone(),
        }
    }

    /// Substitute type arguments into a type definition.
    /// Replaces type parameters (T, E, etc.) with concrete types.
    fn substitute_type_args(&self, def: &Type, args: &[Type]) -> Type {
        // Use substitute_single_type_arg which handles all type forms recursively
        self.substitute_single_type_arg(def, args)
    }

    /// Substitute type parameters in a single type.
    /// Uses the __type_params_ registry to find the parameter positions.
    fn substitute_single_type_arg(&self, ty: &Type, args: &[Type]) -> Type {
        match ty {
            // Type parameter as Var: use index-based substitution
            Type::Var(var) => {
                let var_name = format!("{:?}", var);
                if var_name.contains("T") || var_name.contains("0") {
                    args.first().cloned().unwrap_or_else(|| ty.clone())
                } else if var_name.contains("E") || var_name.contains("1") {
                    args.get(1).cloned().unwrap_or_else(|| ty.clone())
                } else {
                    ty.clone()
                }
            }
            // Type parameter as Named (single-segment, no args) - common for non-variant aliases
            // E.g., in `type IoResult<T> is Result<T, StreamError>`, T is stored as Named("T")
            Type::Named {
                path,
                args: named_args,
            } if named_args.is_empty() => {
                if let Some(ident) = path.as_ident() {
                    let param_name = ident.name.as_str();
                    // Common parameter name patterns: T=0, E=1, A=0, B=1, K=0, V=1
                    let idx = match param_name {
                        "T" | "A" | "K" | "Item" | "Self" => Some(0),
                        "E" | "B" | "V" | "U" => Some(1),
                        "R" | "C" | "W" => Some(2),
                        _ => {
                            // Try single uppercase letter as positional (A=0, B=1, C=2, ...)
                            if param_name.len() == 1
                                && param_name
                                    .chars()
                                    .next()
                                    .is_some_and(|c| c.is_ascii_uppercase())
                            {
                                Some((param_name.as_bytes()[0] - b'A') as usize)
                            } else {
                                None
                            }
                        }
                    };
                    if let Some(i) = idx {
                        if let Some(replacement) = args.get(i) {
                            return replacement.clone();
                        }
                    }
                }
                // Not a type parameter - recurse into args
                let new_args: List<Type> = named_args
                    .iter()
                    .map(|a| self.substitute_single_type_arg(a, args))
                    .collect();
                Type::Named {
                    path: path.clone(),
                    args: new_args,
                }
            }
            // Named types with args - recurse
            Type::Named {
                path,
                args: named_args,
            } => {
                let new_args: List<Type> = named_args
                    .iter()
                    .map(|a| self.substitute_single_type_arg(a, args))
                    .collect();
                Type::Named {
                    path: path.clone(),
                    args: new_args,
                }
            }
            // Recurse into compound types
            Type::Generic {
                name,
                args: inner_args,
            } => {
                // Check if this is a type parameter (no args, single name)
                if inner_args.is_empty() {
                    let param_name = name.as_str();
                    let idx = match param_name {
                        "T" | "A" | "K" | "Item" => Some(0),
                        "E" | "B" | "V" | "U" => Some(1),
                        "R" | "C" | "W" => Some(2),
                        _ => {
                            if param_name.len() == 1
                                && param_name
                                    .chars()
                                    .next()
                                    .is_some_and(|c| c.is_ascii_uppercase())
                            {
                                Some((param_name.as_bytes()[0] - b'A') as usize)
                            } else {
                                None
                            }
                        }
                    };
                    if let Some(i) = idx {
                        if let Some(replacement) = args.get(i) {
                            return replacement.clone();
                        }
                    }
                }
                let new_args: List<Type> = inner_args
                    .iter()
                    .map(|a| self.substitute_single_type_arg(a, args))
                    .collect();
                Type::Generic {
                    name: name.clone(),
                    args: new_args,
                }
            }
            // Recurse into Variant types
            Type::Variant(variants) => {
                let new_variants: indexmap::IndexMap<Text, Type> = variants
                    .iter()
                    .map(|(k, v)| (k.clone(), self.substitute_single_type_arg(v, args)))
                    .collect();
                Type::Variant(new_variants)
            }
            // Recurse into Record types
            Type::Record(fields) => {
                let new_fields: indexmap::IndexMap<Text, Type> = fields
                    .iter()
                    .map(|(k, v)| (k.clone(), self.substitute_single_type_arg(v, args)))
                    .collect();
                Type::Record(new_fields)
            }
            // Recurse into Tuple types
            Type::Tuple(elems) => {
                let new_elems: List<Type> = elems
                    .iter()
                    .map(|e| self.substitute_single_type_arg(e, args))
                    .collect();
                Type::Tuple(new_elems)
            }
            // Recurse into Reference types
            Type::Reference { inner, mutable } => {
                let new_inner = self.substitute_single_type_arg(inner, args);
                Type::Reference {
                    inner: Box::new(new_inner),
                    mutable: *mutable,
                }
            }
            _ => ty.clone(),
        }
    }

    /// Recursively search an expression for a Try (?) operator and extract its error type.
    fn find_try_operator_error_type(&mut self, expr: &Expr) -> Option<Type> {
        use verum_ast::expr::ExprKind;
        use verum_ast::stmt::StmtKind;

        match &expr.kind {
            ExprKind::Try(inner) => {
                // Found a ? operator - use Try protocol to extract error type
                if let Ok(inner_result) = self.synth_expr(inner) {
                    if let Some(resolution) = self
                        .protocol_checker
                        .read()
                        .resolve_try_protocol(&inner_result.ty)
                    {
                        return self
                            .protocol_checker
                            .read()
                            .extract_error_from_residual(&resolution.residual);
                    }
                }
                None
            }

            // Search inside blocks
            ExprKind::Block(block) => self.find_try_operator_error_type_in_block(block),

            // Search inside binary operations
            ExprKind::Binary { left, right, .. } => self
                .find_try_operator_error_type(left)
                .or_else(|| self.find_try_operator_error_type(right)),

            // Search inside if expressions
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Search in condition (it's an IfCondition, which may contain expressions)
                // For simplicity, just search the then/else branches
                self.find_try_operator_error_type_in_block(then_branch)
                    .or_else(|| {
                        else_branch
                            .as_ref()
                            .and_then(|e| self.find_try_operator_error_type(e))
                    })
            }

            // Search inside match expressions
            ExprKind::Match {
                expr: scrutinee,
                arms,
            } => self.find_try_operator_error_type(scrutinee).or_else(|| {
                for arm in arms {
                    if let Some(ty) = self.find_try_operator_error_type(&arm.body) {
                        return Some(ty);
                    }
                }
                None
            }),

            // Field access: search inside `expr.field` for nested ? operators
            ExprKind::Field { expr: inner, .. } => self.find_try_operator_error_type(inner),

            // OptionalChain: `expr?.field` — the lexer tokenizes `?` + `.` as `?.`
            // Inside try-recover blocks, this acts as a ? operator on the inner expr
            ExprKind::OptionalChain { expr: inner, .. } => {
                if let Ok(inner_result) = self.synth_expr(inner) {
                    if let Some(resolution) = self
                        .protocol_checker
                        .read()
                        .resolve_try_protocol(&inner_result.ty)
                    {
                        return self
                            .protocol_checker
                            .read()
                            .extract_error_from_residual(&resolution.residual);
                    }
                }
                None
            }

            // Other expression types - for simplicity, don't search recursively
            _ => None,
        }
    }

    /// Search a block for ? operators
    fn find_try_operator_error_type_in_block(&mut self, block: &verum_ast::Block) -> Option<Type> {
        use verum_ast::stmt::StmtKind;

        // Check statements
        for stmt in &block.stmts {
            match &stmt.kind {
                StmtKind::Expr { expr, .. } => {
                    if let Some(ty) = self.find_try_operator_error_type(expr) {
                        return Some(ty);
                    }
                }
                StmtKind::Let { value: Some(v), .. } => {
                    if let Some(ty) = self.find_try_operator_error_type(v) {
                        return Some(ty);
                    }
                }
                _ => {}
            }
        }

        // Check final expression
        if let Some(ref final_expr) = block.expr {
            return self.find_try_operator_error_type(final_expr);
        }

        None
    }

    /// Infer type for a literal.
    pub(crate) fn infer_literal(&self, lit: &Literal) -> Type {
        use verum_ast::literal::LiteralKind;

        match &lit.kind {
            LiteralKind::Bool(_) => Type::bool(),

            // Integer literals with suffix-based type narrowing
            // Integer type hierarchy: all fixed-size integers (i8..i128, u8..u128) are refinement types of Int with range predicates — .2 lines 143-162
            // Unification: Robinson's algorithm extended with row polymorphism, refinement subtyping, and type class constraints — .4 lines 8664-9060
            LiteralKind::Int(int_lit) => {
                if let Some(ref suffix) = int_lit.suffix {
                    self.infer_int_with_suffix(int_lit.value, suffix, lit.span)
                } else {
                    // No suffix: default to arbitrary-precision Int
                    Type::int()
                }
            }

            // Float literals with suffix-based type narrowing
            LiteralKind::Float(float_lit) => {
                if let Some(ref suffix) = float_lit.suffix {
                    self.infer_float_with_suffix(float_lit.value, suffix, lit.span)
                } else {
                    // No suffix: default to f64
                    Type::float()
                }
            }

            LiteralKind::Char(_) => Type::Char,
            LiteralKind::ByteChar(_) => Type::u8_refined(0),
            LiteralKind::ByteString(_bytes) => {
                // Byte string literal: &[Byte] with known length
                Type::Slice {
                    element: Box::new(Type::u8_refined(0)),
                }
            }
            LiteralKind::Text(_) => Type::text(),
            LiteralKind::Tagged { tag, content: _ } => {
                // Format-specific type inference for tagged literals
                // Spec: grammar/verum.ebnf Section 1.5.2.1 - Tagged Literals
                // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Format Tag Categories
                self.infer_tagged_literal_type(tag.as_str(), lit.span)
            }
            LiteralKind::Contract(_) => Type::text(), // Contract literals are strings
            LiteralKind::ContextAdaptive(_) => Type::text(), // Context-adaptive literals are strings
            LiteralKind::InterpolatedString(_) => Type::text(), // Interpolated strings are strings
            LiteralKind::Composite(comp) => {
                // Infer specific types based on composite literal tags
                // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — #1.4.4 - Composite literals
                match comp.tag.as_str() {
                    // Matrix literal: mat#"[[1,2],[3,4]]"
                    "mat" | "matrix" => {
                        // For now, return a generic matrix type
                        // In production, would parse content to determine dimensions
                        let ident =
                            verum_ast::ty::Ident::new("Matrix", verum_ast::span::Span::dummy());
                        Type::Named {
                            path: Path::single(ident),
                            args: List::new(), // Would contain dimensions as meta parameters
                        }
                    }
                    // Vector literal: vec#"[1,2,3]"
                    "vec" | "vector" => {
                        Type::Named {
                            path: Path::single(verum_ast::ty::Ident::new(
                                "Vector",
                                verum_ast::span::Span::dummy(),
                            )),
                            args: List::new(), // Would contain size as meta parameter
                        }
                    }
                    // Tensor literal: tensor#"[[[1,2],[3,4]],[[5,6],[7,8]]]"
                    // Tensor types: Tensor<T, Shape: meta [usize]> with compile-time shape tracking for N-dimensional arrays
                    "tensor" => {
                        // Parse the tensor literal to extract shape and element type.
                        // The literal string contains a nested array structure.
                        //

                        // Implementation:
                        // 1. Recursively parse nested array structure
                        // 2. Count elements at each nesting level to get actual dimensions
                        // 3. Infer element type from first scalar (Int, Float, Bool, Text)
                        // 4. Validate tensor regularity (all rows must have same length)
                        // 5. Return Tensor<elem_ty, [d1, d2, ..., dn]>
                        //

                        // Example: [[1, 2, 3], [4, 5, 6]] -> Tensor<Int, [2, 3]>
                        let (elem_ty, shape) =
                            self.infer_tensor_literal_structure(comp.content.as_str());

                        Type::tensor(elem_ty, shape, lit.span)
                    }
                    // Interval literal: interval#"[0, 100)"
                    "interval" => {
                        Type::Named {
                            path: Path::single(verum_ast::ty::Ident::new(
                                "Interval",
                                verum_ast::span::Span::dummy(),
                            )),
                            args: List::new(), // Would contain element type
                        }
                    }
                    // Regular expression literal: regex#"[a-z]+", rx#"[a-z]+"
                    // Note: rx is the canonical short form
                    "regex" | "regexp" | "re" | "rx" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "Regex",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // Date/time literal: d#"2024-01-15T10:30:00Z"
                    "d" | "date" | "datetime" | "dt" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "DateTime",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // Duration literal: dur#"3h30m"
                    "dur" | "duration" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            WKT::Duration.as_str(),
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // JSON literal: json#'{"key": "value"}'
                    "json" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "Json",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // XML literal: xml#"<root><child>value</child></root>"
                    "xml" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "Xml",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // YAML literal: yaml#"key: value\nlist:\n - item1\n - item2"
                    "yaml" | "yml" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "Yaml",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // URI/URL literal: uri#"https://example.com/path?query=value"
                    "uri" | "url" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "Uri",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // Email literal: email#"user@example.com"
                    "email" | "mail" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "Email",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // UUID literal: uuid#"550e8400-e29b-41d4-a716-446655440000"
                    "uuid" | "guid" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "Uuid",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // Chemical formula: chem#"H2O"
                    "chem" | "chemical" | "formula" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "ChemicalFormula",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // Music notation: music#"C D E F G A B C"
                    "music" | "note" | "melody" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "MusicNotation",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // SQL query: sql#"SELECT * FROM users WHERE id = {id}"
                    "sql" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "SqlQuery",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // GraphQL query: gql#"query { user(id: $id) { name email } }"
                    "gql" | "graphql" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "GraphQLQuery",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // HTML template: html#"<div>{content}</div>"
                    "html" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "HtmlTemplate",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // CSS styles: css#".class { color: red; }"
                    "css" | "style" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "CssStyles",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },

                    // ================================================================
                    // Additional format tags
                    // Spec: grammar/verum.ebnf - format_tag categories
                    // ================================================================

                    // TOML literal: toml#"[server]\nport = 8080"
                    "toml" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "TomlValue",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },

                    // Network literals
                    // IP address: ip#"192.168.1.1"
                    "ip" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "IpAddr",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // CIDR notation: cidr#"192.168.0.0/16"
                    "cidr" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "CidrRange",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // MAC address: mac#"AA:BB:CC:DD:EE:FF"
                    "mac" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "MacAddr",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // Hostname: host#"example.com"
                    "host" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "Hostname",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },

                    // Pattern matching literals
                    // Glob pattern: glob#"*.txt"
                    "glob" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "GlobPattern",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // XPath expression: xpath#"//book/title"
                    "xpath" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "XPathExpr",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // JSONPath expression: jpath#"$.store.book[*].author"
                    "jpath" | "jsonpath" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "JsonPath",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },

                    // Identifier literals
                    // Path literal: path#"/usr/local/bin"
                    "path" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "PathBuf",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // MIME type: mime#"application/json"
                    "mime" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "MimeType",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // URN literal: urn#"urn:isbn:0451450523"
                    "urn" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "Urn",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },

                    // Temporal literals
                    // Time literal: time#"14:30:00"
                    "time" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "Time",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // Timezone: tz#"America/New_York"
                    "tz" | "timezone" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "Timezone",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },

                    // Version literals
                    // Semantic version: ver#"1.2.3", semver#"1.2.3-beta"
                    "ver" | "semver" | "version" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "Version",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // Base64: b64#"SGVsbG8gV29ybGQ="
                    "b64" | "base64" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "Base64",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // Hexadecimal: hex#"48656c6c6f"
                    "hex" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "HexBytes",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // Percent-encoded: pct#"hello%20world"
                    "pct" | "percent" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "PercentEncoded",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },

                    // Code literals
                    // Shell command: sh#"echo hello"
                    "sh" | "shell" | "bash" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "ShellCommand",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // Lua script: lua#"print('hello')"
                    "lua" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "LuaScript",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // Assembly: asm#"mov eax, 1"
                    "asm" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "Assembly",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },

                    // Science literals
                    // Geographic coordinates: geo#"40.7128,-74.0060"
                    "geo" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "GeoCoord",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },

                    // Query languages
                    // Cypher (Neo4j): cypher#"MATCH (n) RETURN n"
                    "cypher" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "CypherQuery",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },
                    // SPARQL: sparql#"SELECT ?x WHERE { ?x rdf:type ?type }"
                    "sparql" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "SparqlQuery",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },

                    // CSV data: csv#"a,b,c\n1,2,3"
                    "csv" => Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "CsvData",
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    },

                    // Unknown tag - default to Text type
                    _ => Type::Text,
                }
            }
        }
    }

    /// Infer type for integer literal with suffix
    /// Integer type hierarchy: all fixed-size integers (i8..i128, u8..u128) are refinement types of Int with range predicates — .2 lines 143-162
    /// Unification: Robinson's algorithm extended with row polymorphism, refinement subtyping, and type class constraints — .4.2 lines 8705-8754
    ///

    /// Range validation is deferred to refinement checking phase.
    /// This function only performs type narrowing based on the suffix.
    fn infer_int_with_suffix(
        &self,
        _value: i128,
        suffix: &verum_ast::literal::IntSuffix,
        span: Span,
    ) -> Type {
        use verum_ast::literal::IntSuffix;

        match suffix {
            // Signed integer types
            IntSuffix::I8 => Type::i8_refined(0),
            IntSuffix::I16 => Type::i16_refined(0),
            IntSuffix::I32 => Type::i32_refined(0),
            IntSuffix::I64 => Type::i64_refined(0),
            IntSuffix::I128 => Type::i128_refined(0),
            IntSuffix::Isize => Type::isize_refined(0),

            // Unsigned integer types
            IntSuffix::U8 => Type::u8_refined(0),
            IntSuffix::U16 => Type::u16_refined(0),
            IntSuffix::U32 => Type::u32_refined(0),
            IntSuffix::U64 => Type::u64_refined(0),
            IntSuffix::U128 => Type::u128_refined(0),
            IntSuffix::Usize => Type::usize_refined(0),

            // Custom suffixes for units of measure
            // Unification: Robinson's algorithm extended with row polymorphism, refinement subtyping, and type class constraints — .4.2 lines 8710-8744
            IntSuffix::Custom(suffix_str) => {
                // Look up the suffix in the type context to find the appropriate type
                // For now, create a named type based on the suffix
                Type::Named {
                    path: Path::single(verum_ast::ty::Ident::new(suffix_str.as_str(), span)),
                    args: vec![Type::Int].into(),
                }
            }
        }
    }

    /// Infer type for float literal with suffix
    /// Integer type hierarchy: all fixed-size integers (i8..i128, u8..u128) are refinement types of Int with range predicates — .2 lines 143-162
    fn infer_float_with_suffix(
        &self,
        _value: f64,
        suffix: &verum_ast::literal::FloatSuffix,
        _span: Span,
    ) -> Type {
        use verum_ast::literal::FloatSuffix;

        match suffix {
            FloatSuffix::F32 => Type::f32_refined(0.0),
            FloatSuffix::F64 => Type::f64_refined(0.0),
            FloatSuffix::Custom(suffix_str) => {
                // Custom suffixes for units of measure (e.g., "m", "kg", "s")
                Type::Named {
                    path: Path::single(verum_ast::ty::Ident::new(
                        suffix_str.as_str(),
                        verum_ast::span::Span::dummy(),
                    )),
                    args: vec![Type::Float].into(),
                }
            }
        }
    }

    /// Try to resolve an operator through protocol implementation.
    ///

    /// This function looks up protocol implementations (Add, Sub, Mul, Div, etc.)
    /// for the given type without any hardcoded type knowledge.
    ///

    /// ARCHITECTURAL RULE: The type checker MUST NOT have hardcoded knowledge
    /// of stdlib/core types. All operator behavior is discovered from protocol
    /// implementations defined in the source code.
    ///

    /// Returns Some(output_type) if a matching protocol implementation is found,
    /// None if the type doesn't implement the protocol.
    fn try_operator_protocol(
        &mut self,
        left_ty: &Type,
        right: &Expr,
        protocol_name: &str,
        method_name: &str,
        _span: Span,
    ) -> Option<Type> {
        // Get all protocol implementations for this type
        let impls = self
            .protocol_checker
            .read()
            .get_implementations(left_ty)
            .into_iter()
            .cloned()
            .collect::<List<_>>();

        // Look for an implementation of the specified protocol
        for impl_ in impls.iter() {
            // Extract the protocol name from the implementation
            let impl_protocol_name = Self::get_protocol_name_str(&impl_.protocol);
            if impl_protocol_name != protocol_name {
                continue;
            }

            // Found a matching protocol - check if the method exists
            if let Some(method_ty) = impl_.methods.get(&verum_common::Text::from(method_name)) {
                // Extract the return type and parameter types from the method
                if let Type::Function {
                    params,
                    return_type,
                    ..
                } = method_ty
                {
                    // For binary operators, params are [self_type, rhs_type]
                    // Skip self (first param) and get the rhs type
                    let rhs_ty = if params.len() >= 2 {
                        params[1].clone()
                    } else if !impl_.protocol_args.is_empty() {
                        // Get Rhs from protocol args (e.g., Mul<Int> has Int as protocol arg)
                        impl_.protocol_args[0].clone()
                    } else {
                        // Default: same as left type (e.g., Add without params means T + T)
                        left_ty.clone()
                    };

                    // Try to type-check the right operand against the expected Rhs type
                    let rhs_resolved = self.unifier.apply(&rhs_ty);
                    if self.check_expr(right, &rhs_resolved).is_ok() {
                        // Get the output type - either from associated types or return type
                        let output_ty = impl_
                            .associated_types
                            .get(&verum_common::Text::from("Output"))
                            .cloned()
                            .unwrap_or_else(|| return_type.as_ref().clone());

                        // Apply unifier to resolve any type variables
                        return Some(self.unifier.apply(&output_ty));
                    }
                }
            }
        }

        None
    }

    /// Extract protocol name from a Path (e.g., "Mul" from "core.base.protocols.Mul")
    fn get_protocol_name_str(path: &verum_ast::ty::Path) -> &str {
        use verum_ast::ty::PathSegment;
        if let Some(last) = path.segments.last() {
            match last {
                PathSegment::Name(ident) => ident.name.as_str(),
                _ => "",
            }
        } else {
            ""
        }
    }

    /// Try to resolve an operator through protocol implementation using types directly.
    ///

    /// This is the version for iterative inference where we have types, not expressions.
    /// Returns Some(output_type) if a matching protocol implementation is found.
    fn try_operator_protocol_with_types(
        &mut self,
        left_ty: &Type,
        right_ty: &Type,
        protocol_name: &str,
        _method_name: &str,
        _span: Span,
    ) -> Option<Type> {
        // Get all protocol implementations for the left type
        let impls = self
            .protocol_checker
            .read()
            .get_implementations(left_ty)
            .into_iter()
            .cloned()
            .collect::<List<_>>();

        // Look for an implementation of the specified protocol
        for impl_ in impls.iter() {
            let impl_protocol_name = Self::get_protocol_name_str(&impl_.protocol);

            if impl_protocol_name != protocol_name {
                continue;
            }

            // Found a matching protocol - check if the Rhs type matches
            let expected_rhs = if !impl_.protocol_args.is_empty() {
                // Get Rhs from protocol args (e.g., Mul<Int> has Int as the first arg)
                impl_.protocol_args[0].clone()
            } else {
                // Default: same as left type (e.g., Add without params means T + T)
                left_ty.clone()
            };

            // Check if the right type matches the expected Rhs
            let rhs_resolved = self.unifier.apply(&expected_rhs);
            let right_resolved = self.unifier.apply(right_ty);

            // Try to unify (don't mutate on failure)
            if self.types_compatible(&rhs_resolved, &right_resolved) {
                // Get the output type - either from associated types or the left type
                let output_ty = impl_
                    .associated_types
                    .get(&verum_common::Text::from("Output"))
                    .cloned()
                    .unwrap_or_else(|| left_ty.clone());

                return Some(self.unifier.apply(&output_ty));
            }
        }

        None
    }

    /// Check if two types are compatible (for protocol matching)
    fn types_compatible(&self, ty1: &Type, ty2: &Type) -> bool {
        // Pre-check for method disambiguation: reject when CERTAINLY incompatible,
        // accept when uncertain (let real type checker decide).
        match (ty1, ty2) {
            // Type vars are compatible with anything (can be unified later)
            (Type::Var(_), _) | (_, Type::Var(_)) => true,
            // Refinement types: unwrap to base type for compatibility check
            (Type::Refined { base, .. }, other) | (other, Type::Refined { base, .. }) => {
                self.types_compatible(other, base)
            }
            // Auto-borrow compatibility: T is compatible with &T (auto-ref at call site)
            (ty, Type::Reference { inner, .. }) | (Type::Reference { inner, .. }, ty) => {
                self.types_compatible(ty, inner)
            }
            // Same primitives
            (Type::Int, Type::Int) => true,
            (Type::Float, Type::Float) => true,
            (Type::Bool, Type::Bool) => true,
            (Type::Text, Type::Text) => true,
            (Type::Char, Type::Char) => true,
            (Type::Unit, Type::Unit) => true,
            // Int is potentially compatible with sized integer types (coercion at call site)
            (Type::Int, other) | (other, Type::Int) => Self::is_sized_integer_type(other),
            // Float is potentially compatible with Float32 (coercion at call site)
            (Type::Float, other) | (other, Type::Float) => Self::is_float_like_type(other),
            // Primitive vs non-primitive → certainly incompatible
            (Type::Bool | Type::Text | Type::Char | Type::Unit, _) => false,
            (_, Type::Bool | Type::Text | Type::Char | Type::Unit) => false,
            // Named/Generic comparisons — compare type names, normalizing
            // numeric aliases (`u64` ↔ `UInt64`, `i32` ↔ `Int32`, etc.) so
            // that literal-synthesized types match user-declared parameter
            // types regardless of spelling. Without this the pre-check at
            // static-method lookup sites spuriously rejects perfectly
            // legitimate calls and method resolution falls through to the
            // generic "no method found" fallback.
            (Type::Generic { name: n1, .. }, Type::Generic { name: n2, .. }) => {
                Type::canonical_primitive(n1.as_str()) == Type::canonical_primitive(n2.as_str())
            }
            (Type::Named { path: p1, .. }, Type::Named { path: p2, .. }) => {
                Type::canonical_primitive(Self::get_protocol_name_str(p1))
                    == Type::canonical_primitive(Self::get_protocol_name_str(p2))
            }
            (Type::Generic { name, .. }, Type::Named { path, .. })
            | (Type::Named { path, .. }, Type::Generic { name, .. }) => {
                Type::canonical_primitive(name.as_str())
                    == Type::canonical_primitive(Self::get_protocol_name_str(path))
            }
            // For types we can't structurally compare (Array, Tuple, Function, Reference, etc.),
            // assume compatible and let the actual type checker decide.
            _ => true,
        }
    }

    /// Check if a type is a sized integer type that could accept Int literal coercion.
    fn is_sized_integer_type(ty: &Type) -> bool {
        let name = match ty {
            Type::Named { path, .. } => Self::get_protocol_name_str(path),
            Type::Generic { name, .. } => name.as_str(),
            _ => return false,
        };
        // Recognises every sized-integer spelling Verum source can write —
        // both the canonical UpperCamel forms (`Int8`, `UInt64`, `IntSize`)
        // and the lower-case Rust-style aliases (`i8`, `u64`, `usize`) that
        // VCS specs and FFI bindings use freely. Keep these two lists in
        // sync; they are the same set under different spellings.
        matches!(
            name,
            // Canonical names
            "Byte" | "UInt8" | "Int8" | "Int16" | "Int32" | "Int64"
                | "UInt16" | "UInt32" | "UInt64"
                | "ISize" | "USize" | "IntSize" | "UIntSize"
                | "Int128" | "UInt128"
            // Lower-case aliases
                | "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
                | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
        )
    }

    /// Check if a type is a float-like type that could accept Float literal coercion.
    fn is_float_like_type(ty: &Type) -> bool {
        let name = match ty {
            Type::Named { path, .. } => Self::get_protocol_name_str(path),
            Type::Generic { name, .. } => name.as_str(),
            _ => return false,
        };
        matches!(name, "Float32" | "Float64" | "f32" | "f64")
    }

    /// Infer type for binary operation.
    ///

    /// ARCHITECTURAL RULE: This function MUST NOT contain hardcoded knowledge
    /// of stdlib types like Duration, Time, Text, etc. All operator behavior
    /// is discovered through protocol implementations.
    fn infer_binop(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
        _span: Span,
    ) -> Result<InferResult> {
        use BinOp::*;

        match op {
            // Arithmetic operators: handled through protocol lookup
            // - Add protocol for +
            // - Sub protocol for -
            // - Mul protocol for *
            // - Div protocol for /
            // Primitive types (Int, Float) have built-in handling for efficiency.
            // Arithmetic type inference: binary ops produce types based on operand types (Int op Int -> Int, Float op Float -> Float)
            Add | Concat => {
                let left_result = self.synth_expr(left)?;
                let left_ty = Self::deref_for_binop(&left_result.ty);

                // First handle primitive types efficiently
                match left_ty {
                    Type::Int | Type::Float | Type::Text => {
                        self.check_expr(right, left_ty)?;
                        return Ok(InferResult::new(left_ty.clone()));
                    }
                    Type::Var(_) => {
                        let right_result = self.synth_expr(right)?;
                        let right_ty = Self::deref_for_binop(&right_result.ty);
                        self.unifier.unify(left_ty, right_ty, _span)?;
                        return Ok(InferResult::new(right_ty.clone()));
                    }
                    _ => {}
                }

                // Handle numeric literal coercion: if left is a sized integer and right is a literal,
                // try to coerce the literal to the left's type
                let right_is_literal = matches!(&right.kind, ExprKind::Literal(lit)
                    if matches!(lit.kind, verum_ast::literal::LiteralKind::Int(_)));

                if right_is_literal {
                    // Try to check the literal against the left's type (triggers coercion)
                    if self.check_expr(right, left_ty).is_ok() {
                        return Ok(InferResult::new(left_ty.clone()));
                    }
                }

                // Try protocol-based resolution for custom types
                if let Some(output_ty) =
                    self.try_operator_protocol(left_ty, right, "Add", "add", _span)
                {
                    return Ok(InferResult::new(output_ty));
                }

                // No protocol impl - try to unify types
                let right_result = self.synth_expr(right)?;
                let right_ty = Self::deref_for_binop(&right_result.ty);
                if self.unifier.unify(left_ty, right_ty, _span).is_ok() {
                    Ok(InferResult::new(left_ty.clone()))
                } else {
                    // Fall back to Int for backward compatibility
                    self.check_expr(left, &Type::int())?;
                    self.check_expr(right, &Type::int())?;
                    Ok(InferResult::new(Type::int()))
                }
            }

            Sub => {
                let left_result = self.synth_expr(left)?;
                let left_ty = Self::deref_for_binop(&left_result.ty);

                // First handle primitive types efficiently
                match left_ty {
                    Type::Int | Type::Float => {
                        self.check_expr(right, left_ty)?;
                        return Ok(InferResult::new(left_ty.clone()));
                    }
                    Type::Var(_) => {
                        let right_result = self.synth_expr(right)?;
                        let right_ty = Self::deref_for_binop(&right_result.ty);
                        self.unifier.unify(left_ty, right_ty, _span)?;
                        return Ok(InferResult::new(right_ty.clone()));
                    }
                    _ => {}
                }

                // Handle numeric literal coercion: if left is a sized integer and right is a literal,
                // try to coerce the literal to the left's type
                let right_is_literal = matches!(&right.kind, ExprKind::Literal(lit)
                    if matches!(lit.kind, verum_ast::literal::LiteralKind::Int(_)));

                if right_is_literal {
                    // Try to check the literal against the left's type (triggers coercion)
                    if self.check_expr(right, left_ty).is_ok() {
                        return Ok(InferResult::new(left_ty.clone()));
                    }
                }

                // For custom types, synth the right side first to know its type
                // This avoids side effects from speculative check_expr calls
                let right_result = self.synth_expr(right)?;
                let right_ty = Self::deref_for_binop(&right_result.ty);

                // Try protocol-based resolution using types (no side effects)
                if let Some(output_ty) =
                    self.try_operator_protocol_with_types(left_ty, right_ty, "Sub", "sub", _span)
                {
                    return Ok(InferResult::new(output_ty));
                }

                // No protocol impl - try to unify types
                if self.unifier.unify(left_ty, right_ty, _span).is_ok() {
                    Ok(InferResult::new(left_ty.clone()))
                } else {
                    // Fall back to Int for backward compatibility
                    self.check_expr(left, &Type::int())?;
                    self.check_expr(right, &Type::int())?;
                    Ok(InferResult::new(Type::int()))
                }
            }

            Mul | Div | Rem | Pow => {
                let left_result = self.synth_expr(left)?;
                let left_ty = Self::deref_for_binop(&left_result.ty);

                // Map operator to protocol name
                let (protocol_name, method_name) = match op {
                    Mul => ("Mul", "mul"),
                    Div => ("Div", "div"),
                    Rem => ("Rem", "rem"),
                    Pow => ("Pow", "pow"),
                    _ => unreachable!(),
                };

                // First handle primitive types efficiently
                match left_ty {
                    Type::Int | Type::Float => {
                        self.check_expr(right, left_ty)?;
                        return Ok(InferResult::new(left_ty.clone()));
                    }
                    Type::Var(_) => {
                        let right_result = self.synth_expr(right)?;
                        let right_ty = Self::deref_for_binop(&right_result.ty);
                        self.unifier.unify(left_ty, right_ty, _span)?;
                        return Ok(InferResult::new(right_ty.clone()));
                    }
                    _ => {}
                }

                // Check for integer literal coercion for sized numeric types
                let right_is_literal = matches!(&right.kind, ExprKind::Literal(lit)
                    if matches!(lit.kind, verum_ast::literal::LiteralKind::Int(_)));

                if right_is_literal {
                    if self.check_expr(right, left_ty).is_ok() {
                        return Ok(InferResult::new(left_ty.clone()));
                    }
                }

                // Try protocol-based resolution for custom types
                if let Some(output_ty) =
                    self.try_operator_protocol(left_ty, right, protocol_name, method_name, _span)
                {
                    return Ok(InferResult::new(output_ty));
                }

                // No protocol impl - fall back to Int
                self.check_expr(left, &Type::int())?;
                self.check_expr(right, &Type::int())?;
                Ok(InferResult::new(Type::int()))
            }

            // Comparison operators: handled through Ord/PartialOrd protocol lookup
            // Primitive types have built-in handling for efficiency.
            Lt | Le | Gt | Ge => {
                let left_result = self.synth_expr(left)?;
                let left_ty = Self::deref_for_binop(&left_result.ty);

                // First handle primitive types efficiently
                match left_ty {
                    Type::Int | Type::Float => {
                        self.check_expr(right, left_ty)?;
                        return Ok(InferResult::new(Type::bool()));
                    }
                    Type::Var(_) => {
                        let right_result = self.synth_expr(right)?;
                        let right_ty = Self::deref_for_binop(&right_result.ty);
                        self.unifier.unify(left_ty, right_ty, _span)?;
                        return Ok(InferResult::new(Type::bool()));
                    }
                    _ => {}
                }

                // Check for integer literal coercion for sized numeric types
                let right_is_literal = matches!(&right.kind, ExprKind::Literal(lit)
                    if matches!(lit.kind, verum_ast::literal::LiteralKind::Int(_)));

                if right_is_literal {
                    // Try literal coercion: if left is a type that the literal can coerce to
                    if self.check_expr(right, left_ty).is_ok() {
                        return Ok(InferResult::new(Type::bool()));
                    }
                }

                // Try protocol-based resolution (PartialOrd has lt, le, gt, ge methods)
                let method_name = match op {
                    Lt => "lt",
                    Le => "le",
                    Gt => "gt",
                    Ge => "ge",
                    _ => unreachable!(),
                };

                // Check PartialOrd protocol
                if self
                    .try_operator_protocol(left_ty, right, "PartialOrd", method_name, _span)
                    .is_some()
                {
                    return Ok(InferResult::new(Type::bool()));
                }

                // Check Ord protocol (superprotocol of PartialOrd)
                if self
                    .try_operator_protocol(left_ty, right, "Ord", "cmp", _span)
                    .is_some()
                {
                    // Type check the right operand manually since we didn't use a comparison method
                    self.check_expr(right, left_ty)?;
                    return Ok(InferResult::new(Type::bool()));
                }

                // No protocol impl - try to unify types
                let right_result = self.synth_expr(right)?;
                let right_ty = Self::deref_for_binop(&right_result.ty);
                if self.unifier.unify(left_ty, right_ty, _span).is_ok() {
                    Ok(InferResult::new(Type::bool()))
                } else {
                    // Fallback to Int for backward compatibility
                    self.check_expr(left, &Type::int())?;
                    self.check_expr(right, &Type::int())?;
                    Ok(InferResult::new(Type::bool()))
                }
            }

            // Equality: 'a -> 'a -> Bool (auto-deref: &T == T and T == &T both work)
            Eq | Ne => {
                let left_result = self.synth_expr(left)?;
                let right_result = self.synth_expr(right)?;
                // Auto-deref both sides for comparison
                let left_ty = Self::deref_for_binop(&left_result.ty);
                let right_ty = Self::deref_for_binop(&right_result.ty);

                // Handle literal coercion for equality comparisons
                // When comparing a literal Int with a sized type, coerce the literal
                let right_is_literal = matches!(&right.kind, ExprKind::Literal(lit)
                    if matches!(lit.kind, verum_ast::literal::LiteralKind::Int(_)));
                let left_is_literal = matches!(&left.kind, ExprKind::Literal(lit)
                    if matches!(lit.kind, verum_ast::literal::LiteralKind::Int(_)));

                let types_compatible = if right_is_literal && !matches!(left_ty, Type::Int) {
                    // Coerce right literal to left's type
                    self.check_expr(right, left_ty).is_ok()
                } else if left_is_literal && !matches!(right_ty, Type::Int) {
                    // Coerce left literal to right's type
                    self.check_expr(left, right_ty).is_ok()
                } else if self.unifier.unify(left_ty, right_ty, _span).is_ok() {
                    // Same-type: always compatible
                    true
                } else {
                    // Different types: allow if both implement Numeric (cross-numeric coercion)
                    let pc = self.protocol_checker.read();
                    pc.implements_protocol(left_ty, "Numeric")
                        && pc.implements_protocol(right_ty, "Numeric")
                };

                if !types_compatible {
                    return Err(TypeError::Mismatch {
                        expected: self.unifier.apply(left_ty).to_text(),
                        actual: self.unifier.apply(right_ty).to_text(),
                        span: _span,
                    });
                }
                Ok(InferResult::new(Type::bool()))
            }

            // Logical operators: Bool -> Bool -> Bool
            And | Or => {
                self.check_expr(left, &Type::bool())?;
                self.check_expr(right, &Type::bool())?;
                Ok(InferResult::new(Type::bool()))
            }

            // Logical implication: Bool -> Bool -> Bool
            // Used in formal proofs: P -> Q (if P then Q)
            // Formal proof system (future v2.0+): machine-checkable proofs with tactics (simp, ring, omega, blast, induction), theorem/lemma/corollary statements — Logical Implication
            Imply => {
                self.check_expr(left, &Type::bool())?;
                self.check_expr(right, &Type::bool())?;
                Ok(InferResult::new(Type::bool()))
            }

            // Bitwise operators: Int -> Int -> Int (or sized integer types)
            // These operators work on the binary representation of integers
            // Auto-deref: &Int & Int works
            BitAnd | BitOr | BitXor | Shl | Shr => {
                let left_result = self.synth_expr(left)?;
                // Auto-deref for binary operations
                let left_ty = Self::deref_for_binop(&left_result.ty);

                match left_ty {
                    Type::Int => {
                        self.check_expr(right, &Type::int())?;
                        Ok(InferResult::new(Type::int()))
                    }
                    Type::Var(_) => {
                        let right_result = self.synth_expr(right)?;
                        let right_ty = Self::deref_for_binop(&right_result.ty);
                        self.unifier.unify(left_ty, &Type::int(), _span)?;
                        self.unifier.unify(right_ty, &Type::int(), _span)?;
                        Ok(InferResult::new(Type::int()))
                    }
                    _ => {
                        // Try literal coercion first — check_expr validates compatibility
                        let right_is_literal = matches!(&right.kind, ExprKind::Literal(lit)
                            if matches!(lit.kind, verum_ast::literal::LiteralKind::Int(_)));

                        if right_is_literal {
                            if self.check_expr(right, left_ty).is_ok() {
                                return Ok(InferResult::new(left_ty.clone()));
                            }
                        }

                        // Try protocol-based resolution (BitAnd/BitOr/etc.)
                        let (bit_protocol, bit_method) = match op {
                            BitAnd => ("BitAnd", "bitand"),
                            BitOr => ("BitOr", "bitor"),
                            BitXor => ("BitXor", "bitxor"),
                            Shl => ("Shl", "shl"),
                            Shr => ("Shr", "shr"),
                            _ => unreachable!(),
                        };
                        if let Some(output_ty) = self.try_operator_protocol(
                            left_ty,
                            right,
                            bit_protocol,
                            bit_method,
                            _span,
                        ) {
                            return Ok(InferResult::new(output_ty));
                        }

                        // Try to synth right and check compatibility via unification
                        let right_result = self.synth_expr(right)?;
                        let right_ty = Self::deref_for_binop(&right_result.ty);
                        if self.unifier.unify(left_ty, right_ty, _span).is_ok() {
                            return Ok(InferResult::new(left_ty.clone()));
                        }

                        // Fallback: Bitwise operators default to Int
                        self.check_expr(left, &Type::int())?;
                        self.check_expr(right, &Type::int())?;
                        Ok(InferResult::new(Type::int()))
                    }
                }
            }

            // Assignment
            Assign => {
                // =====================================================================
                // DEFINITE ASSIGNMENT: Track assignment BEFORE checking LHS
                // This allows us to skip init checking for assignment targets.
                // Spec: L0-critical/memory-safety/uninitialized
                // =====================================================================
                self.handle_assignment(left, _span);

                // =====================================================================
                // ALIASING CHECK: Assignment to indexed element requires mutable borrow
                // If `data[i] = value`, we need exclusive access to `data`
                // Spec: L0-critical/reference_system/access_rules/ref_conflict_error
                // =====================================================================
                self.check_assignment_aliasing(left, _span)?;

                let right_result = self.synth_expr(right)?;
                // For assignment LHS, use the special method that skips init checking
                self.check_expr_assignment_target(left, &right_result.ty)?;

                Ok(InferResult::new(Type::unit()))
            }

            // Compound assignment operators: +=, -=, *=, /=, %=, &=, |=, ^=, <<=, >>=
            // These desugar to: lhs = lhs op rhs
            // Type check: lhs and rhs must have compatible types for the operation.
            // No hardcoded type name knowledge — types are discovered from
            // protocol implementations registered by the stdlib.
            AddAssign | SubAssign | MulAssign | DivAssign | RemAssign | BitAndAssign
            | BitOrAssign | BitXorAssign | ShlAssign | ShrAssign => {
                let left_result = self.synth_expr(left)?;

                match &left_result.ty {
                    Type::Int | Type::Float => {
                        let right_result = self.synth_expr(right)?;
                        self.unifier
                            .unify(&left_result.ty, &right_result.ty, _span)?;
                    }
                    Type::Text if matches!(op, AddAssign) => {
                        let right_result = self.synth_expr(right)?;
                        self.unifier.unify(&right_result.ty, &Type::text(), _span)?;
                    }
                    Type::Var(_) => {
                        let right_result = self.synth_expr(right)?;
                        self.unifier
                            .unify(&left_result.ty, &right_result.ty, _span)?;
                    }
                    _ => {
                        // For Named and other types: try check_expr for literal
                        // coercion, fall back to synth + unify.
                        if self.check_expr(right, &left_result.ty).is_err() {
                            let right_result = self.synth_expr(right)?;
                            self.unifier
                                .unify(&right_result.ty, &left_result.ty, _span)?;
                        }
                    }
                }
                Ok(InferResult::new(Type::unit()))
            }

            // All BinOp variants are handled above.
            // This fallback is kept for safety - if a new variant is added,
            // this provides a clear error message.
            #[allow(unreachable_patterns)]
            _ => Err(TypeError::Other(verum_common::Text::from(format!(
                "Binary operator {} requires protocol implementation.\n  \
                 Hint: Ensure operand types implement the required protocol (e.g., Add, Sub, Mul, Div)",
                op
            )))),
        }
    }

    /// Infer type for unary operation.
    fn infer_unop(&mut self, op: UnOp, expr: &Expr, _span: Span) -> Result<InferResult> {
        use UnOp::*;

        match op {
            Not => {
                // Not protocol: check via protocol implementation.
                // - Bool: logical NOT, returns Bool
                // - Int: bitwise NOT, returns Int
                // - Named types: check via Not protocol
                let result = self.synth_expr(expr)?;
                match &result.ty {
                    Type::Bool => Ok(InferResult::new(Type::bool())),
                    Type::Int => Ok(InferResult::new(Type::int())),
                    Type::Var(_) => {
                        // Type variable - default to Bool for now
                        self.unifier.unify(&result.ty, &Type::bool(), _span)?;
                        Ok(InferResult::new(Type::bool()))
                    }
                    Type::Named { .. } => {
                        if self
                            .protocol_checker
                            .read()
                            .implements_by_name(&result.ty, "Not")
                            || self.has_method(&result.ty, "not")
                        {
                            Ok(InferResult::new(result.ty.clone()))
                        } else if self.stdlib_single_file_mode {
                            Ok(InferResult::new(Type::Unknown))
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "Cannot apply NOT operator to type: {}. Expected Bool or integer type",
                                result.ty
                            ))))
                        }
                    }
                    _ => {
                        if self.has_method(&result.ty, "not") {
                            return Ok(InferResult::new(result.ty.clone()));
                        }
                        if self.stdlib_single_file_mode {
                            return Ok(InferResult::new(Type::Unknown));
                        }
                        Err(TypeError::Other(verum_common::Text::from(format!(
                            "Cannot apply NOT operator to type: {}. Expected Bool or integer type",
                            result.ty
                        ))))
                    }
                }
            }
            Neg => {
                // Negation: check via Neg protocol or neg method.
                // Refinement types unwrap to their base type for negation.
                let result = self.synth_expr(expr)?;
                match &result.ty {
                    Type::Int => Ok(InferResult::new(Type::int())),
                    Type::Float => Ok(InferResult::new(Type::float())),
                    Type::Refined { base, .. } => {
                        // -Int{>= 0} → Int, -Float{>= 0.0} → Float
                        match base.as_ref() {
                            Type::Int => Ok(InferResult::new(Type::int())),
                            Type::Float => Ok(InferResult::new(Type::float())),
                            _ => Ok(InferResult::new(base.as_ref().clone())),
                        }
                    }
                    Type::Var(_) => {
                        self.unifier.unify(&result.ty, &Type::int(), _span)?;
                        Ok(InferResult::new(Type::int()))
                    }
                    Type::Named { path, .. } => {
                        let name = path
                            .segments
                            .last()
                            .map(|s| match s {
                                verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                                _ => "",
                            })
                            .unwrap_or("");
                        // Signed integers and floats support negation
                        if matches!(
                            name,
                            "Int8"
                                | "Int16"
                                | "Int32"
                                | "Int64"
                                | "I8"
                                | "I16"
                                | "I32"
                                | "I64"
                                | "ISize"
                                | "i8"
                                | "i16"
                                | "i32"
                                | "i64"
                                | "isize"
                                | "F32"
                                | "F64"
                                | "Float32"
                                | "Float64"
                                | "f32"
                                | "f64"
                        ) || self
                            .protocol_checker
                            .read()
                            .implements_by_name(&result.ty, "Neg")
                            || self.has_method(&result.ty, "neg")
                        {
                            Ok(InferResult::new(result.ty.clone()))
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "Cannot negate non-numeric type: {}. Expected Int or Float",
                                result.ty
                            ))))
                        }
                    }
                    _ => {
                        if self.has_method(&result.ty, "neg") {
                            Ok(InferResult::new(result.ty.clone()))
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "Cannot negate non-numeric type: {}. Expected Int or Float",
                                result.ty
                            ))))
                        }
                    }
                }
            }
            BitNot => {
                // Bitwise NOT: check via Not protocol implementation.
                let result = self.synth_expr(expr)?;
                match &result.ty {
                    Type::Int => Ok(InferResult::new(Type::int())),
                    Type::Var(_) => {
                        self.unifier.unify(&result.ty, &Type::int(), _span)?;
                        Ok(InferResult::new(Type::int()))
                    }
                    ty @ (Type::Named { .. } | Type::Generic { .. }) => {
                        let type_name_str = match ty {
                            Type::Named { path, .. } => Self::path_type_name(path)
                                .or_else(|| Self::path_last_type_name(path)),
                            Type::Generic { name, .. } => Some(name.as_str()),
                            _ => None,
                        };
                        let is_int = type_name_str.is_some_and(|n| {
                            matches!(
                                n,
                                "U8" | "U16"
                                    | "U32"
                                    | "U64"
                                    | "U128"
                                    | "USize"
                                    | "I8"
                                    | "I16"
                                    | "I32"
                                    | "I64"
                                    | "I128"
                                    | "ISize"
                                    | "UInt8"
                                    | "UInt16"
                                    | "UInt32"
                                    | "UInt64"
                                    | "Int8"
                                    | "Int16"
                                    | "Int32"
                                    | "Int64"
                            )
                        });
                        if is_int
                            || self
                                .protocol_checker
                                .read()
                                .implements_by_name(&result.ty, "Not")
                        {
                            Ok(InferResult::new(result.ty.clone()))
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "Cannot apply bitwise NOT to non-integer type: {}. Expected integer type",
                                result.ty
                            ))))
                        }
                    }
                    _ => Err(TypeError::Other(verum_common::Text::from(format!(
                        "Cannot apply bitwise NOT to non-integer type: {}. Expected Int",
                        result.ty
                    )))),
                }
            }
            Ref => {
                // CRITICAL: Set call_arg_context so inner path lookup uses borrow_value
                // instead of use_value. Taking &x borrows x, it does NOT consume/move x.
                let old_call_context = self.in_call_arg_context;
                self.in_call_arg_context = true;
                let result = self.synth_expr(expr)?;
                self.in_call_arg_context = old_call_context;

                // Track immutable borrow for aliasing detection
                // Spec: L0-critical/reference_system/access_rules - Reference aliasing
                match &expr.kind {
                    ExprKind::Path(path) => {
                        if let Some(verum_ast::ty::PathSegment::Name(id)) = path.segments.first() {
                            let var_name = id.name.as_str();
                            self.borrow_tracker.borrow_immut(var_name, _span)?;
                        }
                    }
                    ExprKind::Field {
                        expr: receiver,
                        field,
                    } => {
                        // Track field borrow: &container.field borrows container.field
                        // and also implicitly borrows container (prevents &mut container)
                        // Use full path for nested fields: container.first.value -> "first.value"
                        if let Some((base_name, field_path)) =
                            self.extract_field_path(receiver, field.name.as_str())
                        {
                            self.borrow_tracker
                                .borrow_field_immut(base_name, field_path, _span)?;
                        }
                    }
                    // Index expression: &data[i] borrows the element at index i
                    // With constant indices, we can allow disjoint index borrows (borrow splitting)
                    // Spec: L0-critical/reference_system/access_rules/ref_splitting_fields
                    ExprKind::Index {
                        expr: collection,
                        index,
                    } => {
                        if let Some(collection_name) = self.extract_base_name(collection) {
                            // Try to get a constant index for fine-grained tracking
                            if let Some(idx) = self.try_extract_const_index(index) {
                                if idx >= 0 {
                                    // Constant index: track as "collection[idx]" (like a field)
                                    let index_path = verum_common::Text::from(format!("[{}]", idx));
                                    self.borrow_tracker.borrow_field_immut(
                                        verum_common::Text::from(collection_name.as_str()),
                                        index_path,
                                        _span,
                                    )?;
                                }
                                // Negative indices will fail at runtime, but we still track the borrow
                            } else {
                                // Non-constant index: borrow the whole collection
                                self.borrow_tracker
                                    .borrow_immut(collection_name.as_str(), _span)?;
                            }
                        }
                    }
                    _ => {}
                }

                // Auto-deref smart pointers: &Heap<T> -> &T, &Shared<T> -> &T
                // STDLIB-AGNOSTIC: These are Verum memory types, not Rust types
                let inner_ty = match &result.ty {
                    Type::Generic { name, args }
                        if WKT::is_smart_pointer_name(name.as_str()) && args.len() == 1 =>
                    {
                        args[0].clone()
                    }
                    Type::Named { path, args }
                        if Self::path_type_name(path).is_some_and(WKT::is_smart_pointer_name)
                            && args.len() == 1 =>
                    {
                        args[0].clone()
                    }
                    _ => result.ty,
                };
                Ok(InferResult::new(Type::reference(false, inner_ty)))
            }
            RefMut => {
                // CRITICAL: Set call_arg_context so inner path lookup uses borrow_value
                // instead of use_value. Taking &mut x borrows x, it does NOT consume/move x.
                let old_call_context = self.in_call_arg_context;
                self.in_call_arg_context = true;
                let result = self.synth_expr(expr)?;
                self.in_call_arg_context = old_call_context;

                // Track mutable borrow for aliasing detection
                // Spec: L0-critical/reference_system/access_rules - Reference aliasing
                // NLL: Use different behavior depending on context (call arg vs let binding)
                match &expr.kind {
                    ExprKind::Path(path) => {
                        if let Some(verum_ast::ty::PathSegment::Name(id)) = path.segments.first() {
                            let var_name = id.name.as_str();
                            if self.in_call_arg_context {
                                // NLL: For call arguments, use temporary borrow that releases field borrows
                                self.borrow_tracker.borrow_mut_for_call(var_name, _span)?;
                            } else {
                                // Normal: For let bindings, use strict borrow checking
                                self.borrow_tracker.borrow_mut(var_name, _span)?;
                            }
                        }
                    }
                    ExprKind::Field {
                        expr: receiver,
                        field,
                    } => {
                        // Track field borrow: &mut container.field borrows container.field
                        // and also implicitly borrows container (prevents &mut container)
                        // Use full path for nested fields: container.first.value -> "first.value"
                        if let Some((base_name, field_path)) =
                            self.extract_field_path(receiver, field.name.as_str())
                        {
                            self.borrow_tracker
                                .borrow_field_mut(base_name, field_path, _span)?;
                        }
                    }
                    // Index expression: &mut data[i] borrows the element at index i mutably
                    // With constant indices, we can allow disjoint index borrows (borrow splitting)
                    // Spec: L0-critical/reference_system/access_rules/ref_splitting_fields
                    ExprKind::Index {
                        expr: collection,
                        index,
                    } => {
                        if let Some(collection_name) = self.extract_base_name(collection) {
                            // Try to get a constant index for fine-grained tracking
                            if let Some(idx) = self.try_extract_const_index(index) {
                                if idx >= 0 {
                                    // Constant index: track as "collection[idx]" (like a field)
                                    let index_path = verum_common::Text::from(format!("[{}]", idx));
                                    self.borrow_tracker.borrow_field_mut(
                                        verum_common::Text::from(collection_name.as_str()),
                                        index_path,
                                        _span,
                                    )?;
                                }
                                // Negative indices will fail at runtime
                            } else if self.in_call_arg_context {
                                // Non-constant index in call context
                                self.borrow_tracker
                                    .borrow_mut_for_call(collection_name.as_str(), _span)?;
                            } else {
                                // Non-constant index: borrow the whole collection
                                self.borrow_tracker
                                    .borrow_mut(collection_name.as_str(), _span)?;
                            }
                        }
                    }
                    _ => {}
                }

                // Auto-deref smart pointers: &mut Heap<T> -> &mut T, &mut Shared<T> -> &mut T
                // STDLIB-AGNOSTIC: These are Verum memory types, not Rust types
                let inner_ty = match &result.ty {
                    Type::Generic { name, args }
                        if WKT::is_smart_pointer_name(name.as_str()) && args.len() == 1 =>
                    {
                        args[0].clone()
                    }
                    Type::Named { path, args }
                        if Self::path_type_name(path).is_some_and(WKT::is_smart_pointer_name)
                            && args.len() == 1 =>
                    {
                        args[0].clone()
                    }
                    _ => result.ty,
                };
                Ok(InferResult::new(Type::reference(true, inner_ty)))
            }
            Deref => {
                let result = self.synth_expr(expr)?;

                // ============================================================
                // NLL: Release borrow at last use
                // Spec: L0-critical/reference_system/access_rules/ref_scope_valid
                // ============================================================
                // When a reference variable is dereferenced, this is often its
                // last use. For NLL purposes, we release the borrow held by the
                // reference variable, allowing subsequent mutations.
                //

                // Example:
                //  let ref_val = &value;
                //  let read = *ref_val; // Last use - release borrow here
                //  value = 200; // OK: borrow is released
                //

                // This is a conservative NLL approximation that works for simple
                // patterns. Full NLL would require liveness analysis.
                if let ExprKind::Path(path) = &expr.kind {
                    if let Some(verum_ast::ty::PathSegment::Name(id)) = path.segments.first() {
                        let holder_name = id.name.as_str();
                        self.borrow_tracker.release_borrow_at_last_use(holder_name);
                    }
                }

                match result.ty {
                    Type::Reference { inner, .. } => Ok(InferResult::new(*inner)),
                    Type::CheckedReference { inner, .. } => Ok(InferResult::new(*inner)),
                    Type::UnsafeReference { inner, .. } => Ok(InferResult::new(*inner)),
                    // Raw pointer dereference (must be in unsafe block)
                    // CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — #raw-pointer-interop
                    Type::Pointer { inner, .. } => Ok(InferResult::new(*inner)),
                    // Volatile pointer dereference (for MMIO, must be in unsafe block)
                    Type::VolatilePointer { inner, .. } => Ok(InferResult::new(*inner)),

                    // Smart pointer types (Heap, Shared, etc.) dereferenceable to T
                    // Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — Smart pointer types
                    Type::Generic { ref name, ref args } => {
                        // Protocol-based deref: check for Ref<T>/Deref implementation
                        if let Some(target_ty) = self.find_deref_target_type(&result.ty) {
                            Ok(InferResult::new(target_ty))
                        } else if args.len() == 1 {
                            // Fallback: single-arg generic types are likely smart pointer wrappers
                            Ok(InferResult::new(args[0].clone()))
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "Cannot dereference non-reference type: {}.\n  \
                                 Hint: Type must be a reference (&T, &checked T, &unsafe T) or implement Ref<T> protocol.",
                                result.ty
                            ))))
                        }
                    }
                    Type::Named { ref path, ref args } => {
                        // Protocol-based deref: check for Ref<T>/Deref implementation
                        if let Some(target_ty) = self.find_deref_target_type(&result.ty) {
                            Ok(InferResult::new(target_ty))
                        } else if args.len() == 1 {
                            // Fallback: single-arg generic types are likely smart pointer wrappers
                            Ok(InferResult::new(args[0].clone()))
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "Cannot dereference non-reference type: {}.\n  \
                                 Hint: Type must be a reference (&T, &checked T, &unsafe T) or implement Ref<T> protocol.",
                                result.ty
                            ))))
                        }
                    }

                    // ============================================================
                    // Auto-deref through protocol bounds
                    // Spec: L0-critical/reference_system/reference_tiers/tier_conversion.vr
                    // ============================================================
                    // When `*r` is used on a type variable `R` with a `Ref<T>` bound,
                    // the dereference returns `T` (the target type from the protocol).
                    //

                    // This enables generic code like:
                    //  fn read_generic<R: Ref<T>, T>(r: R) -> T { *r }
                    //

                    // At runtime, `*r` calls `r.deref()` from the Ref protocol.
                    Type::Var(var) => {
                        // First, try to resolve the type variable through unification
                        // to see if it's been bound to a concrete type
                        let resolved = self.unifier.apply(&Type::Var(var));
                        // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG deref] Type::Var(T{}) resolved to {:?}", var.id(), resolved);

                        // If the type variable resolved to a reference type, use that
                        match &resolved {
                            Type::Reference { inner, .. } => {
                                return Ok(InferResult::new(*inner.clone()));
                            }
                            Type::CheckedReference { inner, .. } => {
                                return Ok(InferResult::new(*inner.clone()));
                            }
                            Type::UnsafeReference { inner, .. } => {
                                return Ok(InferResult::new(*inner.clone()));
                            }
                            Type::Pointer { inner, .. } => {
                                return Ok(InferResult::new(*inner.clone()));
                            }
                            Type::VolatilePointer { inner, .. } => {
                                return Ok(InferResult::new(*inner.clone()));
                            }
                            _ => {}
                        }

                        // Check if type variable has Ref<T> or RefMut<T> bound
                        let bounds = self.get_type_var_bounds(&var);
                        for bound in &bounds {
                            if let Some(ident) = bound.protocol.as_ident() {
                                let proto_name = ident.name.as_str();
                                if proto_name == "Ref" || proto_name == "RefMut" {
                                    // Extract T from Ref<T> or RefMut<T>
                                    if let Some(target_ty) = bound.args.first() {
                                        return Ok(InferResult::new(target_ty.clone()));
                                    }
                                }
                            }
                        }
                        // No Ref/RefMut bound found - produce a fresh type variable
                        // and constrain the original to be a reference to it.
                        // This handles deferred resolution (e.g., Iterator.next() returning Maybe<&T>).
                        let result_var = TypeVar::fresh();
                        let ref_ty = Type::Reference {
                            inner: Box::new(Type::Var(result_var)),
                            mutable: false,
                        };
                        let resolved = self.unifier.apply(&Type::Var(var));
                        let _ = self.unifier.unify(&resolved, &ref_ty, expr.span);
                        Ok(InferResult::new(Type::Var(result_var)))
                    }

                    // Check for concrete types implementing Ref<T>
                    ref ty => {
                        // Try to find a Ref<T> implementation for this type
                        if let Some(target_ty) = self.find_deref_target_type(ty) {
                            Ok(InferResult::new(target_ty))
                        } else {
                            // Transparent deref: `*x` on a value type is identity.
                            // This handles iterator patterns where items may be resolved
                            // as values rather than references.
                            Ok(InferResult::new(ty.clone()))
                        }
                    }
                }
            }
            // Three-tier reference system: &checked T (Tier 1 - compiler-verified, 0ns overhead)
            // Spec: L0-critical/reference_system/reference_tiers/checked_promotion_fail
            RefChecked => {
                let old_call_context = self.in_call_arg_context;
                self.in_call_arg_context = true;
                let result = self.synth_expr(expr)?;
                self.in_call_arg_context = old_call_context;

                // Track immutable borrow for aliasing detection and escape analysis
                // Same tracking as regular &T, needed for interprocedural escape detection
                match &expr.kind {
                    ExprKind::Path(path) => {
                        if let Some(verum_ast::ty::PathSegment::Name(id)) = path.segments.first() {
                            let var_name = id.name.as_str();
                            self.borrow_tracker.borrow_immut(var_name, _span)?;
                        }
                    }
                    _ => {}
                }

                Ok(InferResult::new(Type::checked_reference(false, result.ty)))
            }
            RefCheckedMut => {
                let old_call_context = self.in_call_arg_context;
                self.in_call_arg_context = true;
                let result = self.synth_expr(expr)?;
                self.in_call_arg_context = old_call_context;

                // Track mutable borrow for aliasing detection and escape analysis
                match &expr.kind {
                    ExprKind::Path(path) => {
                        if let Some(verum_ast::ty::PathSegment::Name(id)) = path.segments.first() {
                            let var_name = id.name.as_str();
                            self.borrow_tracker.borrow_mut(var_name, _span)?;
                        }
                    }
                    _ => {}
                }

                Ok(InferResult::new(Type::checked_reference(true, result.ty)))
            }
            // Three-tier reference system: &unsafe T (Tier 2 - manual proof, 0ns overhead)
            // Requires unsafe block context
            // Spec: L0-critical/reference_system/reference_tiers/unsafe_without_block
            RefUnsafe => {
                // Check that we're in an unsafe context
                // Exception: allow &unsafe TypeName for type-level property access
                // (e.g., (&unsafe Int).size) since no actual reference is created
                let is_type_expr = match &expr.kind {
                    ExprKind::Path(path) if path.segments.len() == 1 => {
                        if let Some(verum_ast::ty::PathSegment::Name(id)) = path.segments.first() {
                            let tn = id.name.as_str();
                            verum_common::well_known_types::type_names::is_primitive_value_type(tn)
                                || verum_common::well_known_types::type_names::is_numeric_type(tn)
                                || matches!(tn, "Text" | "Duration")
                                || self.ctx.env.lookup(tn).is_none()
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                if !self.in_unsafe_context && !is_type_expr {
                    return Err(TypeError::Other(verum_common::Text::from(
                        "unsafe reference requires unsafe block: `unsafe { &unsafe expr }`",
                    )));
                }
                let old_call_context = self.in_call_arg_context;
                self.in_call_arg_context = true;
                let result = self.synth_expr(expr)?;
                self.in_call_arg_context = old_call_context;
                Ok(InferResult::new(Type::unsafe_reference(false, result.ty)))
            }
            RefUnsafeMut => {
                // Check that we're in an unsafe context
                if !self.in_unsafe_context {
                    return Err(TypeError::Other(verum_common::Text::from(
                        "unsafe mutable reference requires unsafe block: `unsafe { &unsafe mut expr }`",
                    )));
                }
                let old_call_context = self.in_call_arg_context;
                self.in_call_arg_context = true;
                let result = self.synth_expr(expr)?;
                self.in_call_arg_context = old_call_context;
                Ok(InferResult::new(Type::unsafe_reference(true, result.ty)))
            }
            // Ownership operators: %x and %mut x
            // These take a reference type and return an ownership reference (Tier 2 unsafe reference)
            // CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — Ownership References
            Own => {
                let result = self.synth_expr(expr)?;
                // Ownership reference returns a Tier 2 unsafe reference (immutable)
                Ok(InferResult::new(Type::unsafe_reference(false, result.ty)))
            }
            OwnMut => {
                let result = self.synth_expr(expr)?;
                // Ownership mutable reference returns a Tier 2 unsafe reference (mutable)
                Ok(InferResult::new(Type::unsafe_reference(true, result.ty)))
            }
            // All UnaryOp variants are handled above.
            // This fallback is kept for safety - if a new variant is added,
            // this provides a clear error message.
            #[allow(unreachable_patterns)]
            _ => Err(TypeError::Other(verum_common::Text::from(format!(
                "Unary operator {} requires protocol implementation.\n  \
                 Hint: Ensure operand type implements the required protocol",
                op
            )))),
        }
    }

    /// Extract the base variable name from a field access expression.
    /// Used for field-level borrow tracking.
    ///

    /// Examples:
    /// - `container.field` -> Some("container")
    /// - `a.b.c` -> Some("a") (extracts root receiver)
    /// - `(expr).field` -> None (complex expressions not tracked)
    fn extract_receiver_name(&self, receiver: &Expr) -> Option<Text> {
        match &receiver.kind {
            ExprKind::Path(path) => {
                // Simple variable: container.field
                if let Some(verum_ast::ty::PathSegment::Name(id)) = path.segments.first() {
                    Some(verum_common::Text::from(id.name.as_str()))
                } else {
                    None
                }
            }
            ExprKind::Field {
                expr: inner_receiver,
                ..
            } => {
                // Nested field access: a.b.c -> extract "a"
                self.extract_receiver_name(inner_receiver)
            }
            _ => {
                // Complex expressions (function calls, etc.) not tracked
                None
            }
        }
    }

    /// Extract implicit type parameters from an AST type.
    /// For types like `fn(I.Item) -> U`, finds uppercase identifiers that aren't
    /// already registered as type variables (like `U`) and registers them.
    fn extract_implicit_type_params_from_type(
        &mut self,
        ty: &verum_ast::ty::Type,
        type_param_names: &mut List<Text>,
    ) {
        use verum_ast::ty::TypeKind;
        match &ty.kind {
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let name = ident.name.as_str();
                    // Uppercase single letters or short names that aren't registered
                    if name
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false)
                        && name.len() <= 3
                        && self.ctx.lookup_type(name).is_none()
                        && !matches!(
                            name,
                            "Int" | "Float" | "Bool" | "Char" | "Text" | "Unit" | "Never" | "Byte"
                        )
                    {
                        let type_var = Type::Var(TypeVar::fresh());
                        let name_text: Text = name.into();
                        self.ctx.define_type(name_text.clone(), type_var);
                        type_param_names.push(name_text);
                    }
                }
            }
            TypeKind::Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    self.extract_implicit_type_params_from_type(param, type_param_names);
                }
                self.extract_implicit_type_params_from_type(return_type, type_param_names);
            }
            TypeKind::Generic { base, args } => {
                self.extract_implicit_type_params_from_type(base, type_param_names);
                for arg in args {
                    if let verum_ast::ty::GenericArg::Type(t) = arg {
                        self.extract_implicit_type_params_from_type(t, type_param_names);
                    }
                }
            }
            TypeKind::Tuple(elems) => {
                for elem in elems {
                    self.extract_implicit_type_params_from_type(elem, type_param_names);
                }
            }
            TypeKind::Reference { inner, .. } => {
                self.extract_implicit_type_params_from_type(inner, type_param_names);
            }
            TypeKind::Qualified {
                self_ty,
                assoc_name: _,
                trait_ref: _,
            } => {
                self.extract_implicit_type_params_from_type(self_ty, type_param_names);
            }
            _ => {}
        }
    }

    /// Extract the base variable name from any expression, including index access.
    /// Used for aliasing detection when borrowing collection elements.
    ///

    /// Examples:
    /// - `data` -> Some("data")
    /// - `data[0]` -> Some("data")
    /// - `data[i][j]` -> Some("data")
    /// - `container.field` -> Some("container")
    /// - `container.field[i]` -> Some("container")
    /// - Spec: L0-critical/reference_system/access_rules/ref_conflict_error
    fn extract_base_name(&self, expr: &Expr) -> Option<Text> {
        match &expr.kind {
            ExprKind::Path(path) => {
                // Simple variable: data
                if let Some(verum_ast::ty::PathSegment::Name(id)) = path.segments.first() {
                    Some(verum_common::Text::from(id.name.as_str()))
                } else {
                    None
                }
            }
            ExprKind::Field { expr: receiver, .. } => {
                // Field access: container.field -> "container"
                self.extract_base_name(receiver)
            }
            ExprKind::Index {
                expr: collection, ..
            } => {
                // Index access: data[i] -> "data"
                self.extract_base_name(collection)
            }
            _ => {
                // Complex expressions not tracked
                None
            }
        }
    }

    /// Extract the full field path from a field access expression.
    /// Returns (base_name, full_path) for field-level borrow tracking.
    ///

    /// Examples:
    /// - `container.field` -> Some(("container", "field"))
    /// - `container.first.value` -> Some(("container", "first.value"))
    /// - `a.b.c.d` -> Some(("a", "b.c.d"))
    fn extract_field_path(&self, receiver: &Expr, field: &str) -> Option<(Text, Text)> {
        match &receiver.kind {
            ExprKind::Path(path) => {
                // Simple variable: container.field -> ("container", "field")
                if let Some(verum_ast::ty::PathSegment::Name(id)) = path.segments.first() {
                    Some((
                        verum_common::Text::from(id.name.as_str()),
                        verum_common::Text::from(field),
                    ))
                } else {
                    None
                }
            }
            ExprKind::Field {
                expr: inner_receiver,
                field: inner_field,
            } => {
                // Nested field access: container.first.value
                // Recursively build the path
                if let Some((base, inner_path)) =
                    self.extract_field_path(inner_receiver, inner_field.name.as_str())
                {
                    // Append current field: "first" + "value" -> "first.value"
                    let full_path = verum_common::Text::from(format!("{}.{}", inner_path, field));
                    Some((base, full_path))
                } else {
                    None
                }
            }
            _ => {
                // Complex expressions not tracked
                None
            }
        }
    }

    /// Infer type for type property expressions.
    /// Spec: grammar/verum.ebnf Section 2.17 - Type Properties
    ///

    /// Type properties provide compile-time access to type metadata:
    /// - `T.size` -> Int (size in bytes)
    /// - `T.alignment` -> Int (alignment in bytes)
    /// - `T.stride` -> Int (memory stride)
    /// - `T.min` -> T (minimum value, only for numeric types)
    /// - `T.max` -> T (maximum value, only for numeric types)
    /// - `T.bits` -> Int (bit width, only for sized types)
    /// - `T.name` -> Text (type name as string)
    fn infer_type_property(
        &mut self,
        ty: &verum_ast::ty::Type,
        property: &TypeProperty,
        span: Span,
    ) -> Result<InferResult> {
        // Convert the AST type to our internal Type representation
        let resolved_ty = self.ast_to_type(ty)?;

        // Handle type variables for generic type parameters
        // These will be resolved at monomorphization time
        if matches!(&resolved_ty, Type::Var(_)) {
            match property {
                // These properties always return Int (including Id which is u64)
                TypeProperty::Size
                | TypeProperty::Alignment
                | TypeProperty::Stride
                | TypeProperty::Bits
                | TypeProperty::Id => Ok(InferResult::new(Type::int())),
                // min/max return the type itself
                TypeProperty::Min | TypeProperty::Max => Ok(InferResult::new(resolved_ty)),
                // name returns Text
                TypeProperty::Name => Ok(InferResult::new(Type::text())),
            }
        } else {
            // Concrete type - validate and return appropriate result type
            match property {
                // Size, alignment, and stride are always valid for any type
                TypeProperty::Size | TypeProperty::Alignment | TypeProperty::Stride => {
                    Ok(InferResult::new(Type::int()))
                }

                // Bits is valid for Numeric types (sized integers and floats)
                // and built-in `Bool` (stored as 1 byte = 8 bits) /
                // `Char` (UTF-32 code point = 32 bits).
                //

                // The protocol-implements check covers stdlib-defined Numeric
                // types; we additionally short-circuit on the lexer's built-in
                // sized integer / float spellings (`Int8`, `UInt32`, `IntSize`,
                // `i8`, `u64`, `usize`, `f32`, …) so VCS specs like
                // `vbc/micro/type_properties/{int_bits,builtin_properties}.vr`
                // typecheck without needing each alias to explicitly
                // `implement Numeric`.
                TypeProperty::Bits => {
                    if matches!(
                        resolved_ty,
                        Type::Int | Type::Float | Type::Bool | Type::Char
                    ) || Self::is_sized_integer_type(&resolved_ty)
                        || Self::is_float_like_type(&resolved_ty)
                        || self
                            .protocol_checker
                            .read()
                            .implements_protocol(&resolved_ty, "Numeric")
                    {
                        Ok(InferResult::new(Type::int()))
                    } else {
                        Err(TypeError::Other(verum_common::Text::from(format!(
                            "Type property 'bits' is only valid for sized numeric types, but got '{}'.\n  \
                             Hint: Use '.bits' with types like Int, Float, Bool, Char, i8, i16, i32, i64, u8, u16, u32, u64, f32, f64",
                            resolved_ty
                        ))))
                    }
                }

                // Min and max are valid for Numeric types
                TypeProperty::Min | TypeProperty::Max => {
                    if matches!(resolved_ty, Type::Int | Type::Float)
                        || Self::is_sized_integer_type(&resolved_ty)
                        || Self::is_float_like_type(&resolved_ty)
                        || self
                            .protocol_checker
                            .read()
                            .implements_protocol(&resolved_ty, "Numeric")
                    {
                        Ok(InferResult::new(resolved_ty))
                    } else {
                        Err(TypeError::Other(verum_common::Text::from(format!(
                            "Type property '{}' is only valid for numeric types, but got '{}'.\n  \
                             Hint: Use '.{}' with types like Int, Float, or sized integer/float types",
                            property, resolved_ty, property
                        ))))
                    }
                }

                // Name is valid for any type
                TypeProperty::Name => Ok(InferResult::new(Type::text())),

                // Id returns u64 hash of type name, valid for any type
                TypeProperty::Id => Ok(InferResult::new(Type::int())),
            }
        }
    }

    /// Coerce float types: when one is Float and the other is Float64/Float32,
    /// return the sized type (Float64/Float32). This enables mixing float literals
    /// (which default to Float) with sized float variables.
    fn coerce_float_types(&self, left: &Type, right: &Type) -> Option<Type> {
        let left_name = self.get_type_name(left);
        let right_name = self.get_type_name(right);

        match (left_name.as_deref(), right_name.as_deref()) {
            // Float + Float64 -> Float64
            (Some("Float"), Some("Float64" | "f64")) => Some(right.clone()),
            (Some("Float64" | "f64"), Some("Float")) => Some(left.clone()),
            // Float + Float32 -> Float32
            (Some("Float"), Some("Float32" | "f32")) => Some(right.clone()),
            (Some("Float32" | "f32"), Some("Float")) => Some(left.clone()),
            // Same type - no coercion needed
            _ => None,
        }
    }

    /// Find the deref target type for a concrete type implementing Ref<T>.
    ///

    /// Spec: L0-critical/reference_system/reference_tiers/tier_conversion.vr
    ///

    /// This enables auto-deref for types that implement the Ref<T> protocol.
    /// When `*x` is used on a type implementing `Ref<T>`, it returns `T`.
    ///

    /// Built-in dereferenceable types (Heap<T>, Shared<T>) are handled here
    /// as well as user-defined types implementing Ref<T>.
    fn find_deref_target_type(&self, ty: &Type) -> Option<Type> {
        // Protocol-based deref: query Ref<T> protocol implementations.
        // No hardcoded type names — all dereferenceable types must implement Ref<T>.
        self.find_ref_protocol_target(ty)
    }

    /// Find the target type T if a type implements Ref<T> protocol.
    ///

    /// Queries the protocol checker for Ref<T> implementation on the given type.
    fn find_ref_protocol_target(&self, ty: &Type) -> Option<Type> {
        // Look for Ref<T> implementation on this type
        // The protocol checker tracks implementations registered via `implement Ref<T> for SomeType`
        let protocol_checker_guard = self.protocol_checker.read();
        let impls = protocol_checker_guard.get_implementations(ty);
        for impl_ in impls.iter() {
            // Extract protocol name from Path
            let protocol_name = impl_
                .protocol
                .as_ident()
                .map(|id| id.name.as_str())
                .unwrap_or("");

            if protocol_name == "Ref" || protocol_name == "RefMut" {
                // Found Ref<T> implementation - the target type is the first protocol argument
                if let Some(target) = impl_.protocol_args.first() {
                    return Some(target.clone());
                }
            }
            // Also support Deref/DerefMut protocols (associated type Target)
            if protocol_name == "Deref" || protocol_name == "DerefMut" {
                if let Some(target) = impl_
                    .associated_types
                    .get(&verum_common::Text::from("Target"))
                {
                    return Some(target.clone());
                }
            }
        }
        None
    }

    /// Convert AST type bounds to protocol bounds.
    ///

    /// Converts from verum_ast::ty::TypeBound to crate::protocol::ProtocolBound.
    /// This is used when processing where clauses and generic parameter bounds.
    ///

    /// Example: `T: Clone + Display` -> [ProtocolBound(Clone), ProtocolBound(Display)]
    fn convert_type_bounds_to_protocol_bounds(
        &mut self,
        bounds: &[verum_ast::ty::TypeBound],
    ) -> Result<List<crate::protocol::ProtocolBound>> {
        let mut protocol_bounds = List::new();

        for bound in bounds {
            use verum_ast::ty::TypeBoundKind;
            match &bound.kind {
                TypeBoundKind::Protocol(path) => {
                    // Simple protocol bound: T: Clone
                    protocol_bounds.push(crate::protocol::ProtocolBound {
                        protocol: path.clone(),
                        args: List::new(),
                        is_negative: false,
                    });
                }
                TypeBoundKind::Equality(_ty) => {
                    // Equality bounds (T = SomeType) are handled differently
                    // For now, we skip them in protocol bounds
                }
                TypeBoundKind::NegativeProtocol(path) => {
                    // Negative bounds (T: !Protocol) are used for specialization
                    // Multi-protocol bounds: combining multiple protocol constraints (T: Display + Debug) — Negative Bounds
                    //

                    // When T: !Protocol, we require that T does NOT implement Protocol.
                    // This enables patterns like:
                    //  implement<T: Clone + !Copy> DeepClone for T { ... }
                    // which only applies to types that are Clone but NOT Copy.
                    protocol_bounds.push(crate::protocol::ProtocolBound {
                        protocol: path.clone(),
                        args: List::new(),
                        is_negative: true,
                    });
                }
                TypeBoundKind::AssociatedTypeBound { .. } => {
                    // Associated type bounds: T.Item: Display
                    // These are handled separately in the type system
                }
                TypeBoundKind::AssociatedTypeEquality { .. } => {
                    // Associated type equality: T.Item = String
                    // These create type equality constraints
                }
                TypeBoundKind::GenericProtocol(ty) => {
                    // Generic protocol bound: Iterator<Item = T>
                    // Extract the base protocol path from the generic type
                    use verum_ast::ty::TypeKind;
                    if let TypeKind::Generic { base, args } = &ty.kind {
                        if let TypeKind::Path(path) = &base.kind {
                            // Convert generic args to protocol bound args
                            let bound_args: List<Type> = args
                                .iter()
                                .filter_map(|arg| {
                                    if let verum_ast::ty::GenericArg::Type(t) = arg {
                                        Some(self.ast_to_type_lenient(t))
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            protocol_bounds.push(crate::protocol::ProtocolBound {
                                protocol: path.clone(),
                                args: bound_args,
                                is_negative: false,
                            });
                        }
                    }
                }
            }
        }

        Ok(protocol_bounds)
    }

    /// Extract direct type bounds from AST bounds.
    ///

    /// Converts Equality bounds (like `F: fn() -> T`) to Type values.
    /// These are bounds that represent actual types, not protocol references.
    /// Used alongside `convert_type_bounds_to_protocol_bounds` to fully capture
    /// all bound information for type variables.
    ///

    /// # Arguments
    /// * `bounds` - The AST type bounds to process
    ///

    /// # Returns
    /// A list of Types representing direct type bounds (function types, etc.)
    fn extract_type_bounds_from_ast(&mut self, bounds: &[verum_ast::ty::TypeBound]) -> List<Type> {
        let mut type_bounds = List::new();

        for bound in bounds {
            use verum_ast::ty::TypeBoundKind;
            // #[cfg(debug_assertions)]
            // eprintln!("[DEBUG extract_type_bounds_from_ast] Processing bound kind: {:?}", std::mem::discriminant(&bound.kind));

            match &bound.kind {
                TypeBoundKind::Equality(ty) => {
                    // #[cfg(debug_assertions)]
                    // eprintln!("[DEBUG extract_type_bounds_from_ast] Found Equality bound with type kind: {:?}", std::mem::discriminant(&ty.kind));
                    // Equality bound: F: fn() -> T or T = SomeType
                    // Convert the AST type to our internal Type representation
                    if let Ok(converted) = self.ast_to_type(ty) {
                        // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG extract_type_bounds_from_ast] Converted to: {}", converted.to_text());
                        type_bounds.push(converted);
                    } else {
                        // Fallback to lenient conversion
                        type_bounds.push(self.ast_to_type_lenient(ty));
                    }
                }
                TypeBoundKind::GenericProtocol(ty) => {
                    // Also handle generic bounds that might be function types
                    // e.g., `F: Fn(T) -> U` where Fn is a generic type
                    use verum_ast::ty::TypeKind;
                    if let TypeKind::Function { .. } = &ty.kind {
                        if let Ok(converted) = self.ast_to_type(ty) {
                            type_bounds.push(converted);
                        }
                    }
                }
                _ => {
                    // #[cfg(debug_assertions)]
                    // eprintln!("[DEBUG extract_type_bounds_from_ast] Skipping non-Equality bound");
                    // Protocol bounds, negative bounds, etc. are handled by
                    // convert_type_bounds_to_protocol_bounds
                }
            }
        }

        type_bounds
    }

    /// Convert AST generic parameters to protocol type parameters.
    ///

    /// Transforms verum_ast::ty::GenericParam list into crate::protocol::TypeParam list.
    /// Used when registering generic protocols like `type Iterator<T> is protocol { ... }`.
    ///

    /// # Examples
    /// - `<T>` -> TypeParam { name: "T", bounds: [], default: None }
    /// - `<T: Clone>` -> TypeParam { name: "T", bounds: [Clone], default: None }
    /// - `<T = Int>` -> TypeParam { name: "T", bounds: [], default: Some(Int) }
    fn convert_generic_params_to_type_params(
        &mut self,
        generics: &[verum_ast::ty::GenericParam],
    ) -> List<crate::protocol::TypeParam> {
        use verum_ast::ty::GenericParamKind;

        let mut type_params = List::new();

        for generic_param in generics {
            match &generic_param.kind {
                GenericParamKind::Type {
                    name,
                    bounds,
                    default,
                } => {
                    // Convert bounds from TypeBound to ProtocolBound
                    let protocol_bounds = self
                        .convert_type_bounds_to_protocol_bounds(bounds)
                        .unwrap_or_else(|_| List::new());

                    // Convert default type if present
                    let default_type = match default {
                        Some(ty) => match self.ast_to_type(ty) {
                            Ok(t) => Maybe::Some(t),
                            Err(_) => Maybe::None,
                        },
                        None => Maybe::None,
                    };

                    type_params.push(crate::protocol::TypeParam {
                        name: verum_common::Text::from(name.name.as_str()),
                        bounds: protocol_bounds,
                        default: default_type,
                    });
                }
                GenericParamKind::HigherKinded { name, bounds, .. } => {
                    // Higher-kinded type parameters like F<_>
                    // Convert bounds similarly
                    let protocol_bounds = self
                        .convert_type_bounds_to_protocol_bounds(bounds)
                        .unwrap_or_else(|_| List::new());

                    type_params.push(crate::protocol::TypeParam {
                        name: verum_common::Text::from(name.name.as_str()),
                        bounds: protocol_bounds,
                        default: Maybe::None,
                    });
                }
                GenericParamKind::Const { name, .. } => {
                    // Const generic parameters - store name without bounds for now
                    type_params.push(crate::protocol::TypeParam {
                        name: verum_common::Text::from(name.name.as_str()),
                        bounds: List::new(),
                        default: Maybe::None,
                    });
                }
                GenericParamKind::Lifetime { name, .. } => {
                    // Lifetime parameters - store name only
                    type_params.push(crate::protocol::TypeParam {
                        name: verum_common::Text::from(name.name.as_str()),
                        bounds: List::new(),
                        default: Maybe::None,
                    });
                }
                GenericParamKind::Meta { name, .. } => {
                    // Meta parameters (compile-time values) - store name only
                    type_params.push(crate::protocol::TypeParam {
                        name: verum_common::Text::from(name.name.as_str()),
                        bounds: List::new(),
                        default: Maybe::None,
                    });
                }
                GenericParamKind::Context { name } => {
                    // Context parameters for context polymorphism
                    // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 17.2
                    // Context parameters represent context requirements that depend on callbacks
                    type_params.push(crate::protocol::TypeParam {
                        name: verum_common::Text::from(name.name.as_str()),
                        bounds: List::new(),
                        default: Maybe::None,
                    });
                }
                GenericParamKind::Level { name } => {
                    // Universe level parameters for universe polymorphism
                    type_params.push(crate::protocol::TypeParam {
                        name: verum_common::Text::from(name.name.as_str()),
                        bounds: List::new(),
                        default: Maybe::None,
                    });
                }
                GenericParamKind::KindAnnotated { name, bounds, .. } => {
                    // Kind-annotated HKT parameter: F: Type -> Type
                    // Treated like HigherKinded for protocol purposes.
                    let protocol_bounds = self
                        .convert_type_bounds_to_protocol_bounds(bounds)
                        .unwrap_or_else(|_| List::new());
                    type_params.push(crate::protocol::TypeParam {
                        name: verum_common::Text::from(name.name.as_str()),
                        bounds: protocol_bounds,
                        default: Maybe::None,
                    });
                }
            }
        }

        type_params
    }
}
