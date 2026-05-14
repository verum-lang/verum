//! TypeChecker construction and stdlib bootstrap methods.
//!
//! Contains the primary constructor (`new`), stdlib-loading entry points,
//! metadata-driven inherent-method registration, and the core factory methods
//! (`with_minimal_context`, `new_with_core`, `new_with_core_eager`).

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
use crate::{Result, TypeError, TypeCheckMetrics};
#[allow(unused_imports)]
use super::{
    TypeChecker, FunctionContract, DeferredConstraint, DeferredVerificationGoal,
    GeneratorContext, InferenceDepthGuard, InferMode, InferResult, InferWork,
    GlobalDepthGuard, ThreadLocalDepthGuard, TypeResolutionCycleGuard, NormalizeTypeCycleGuard,
    WKT_HEAP, WKT_RESULT, WKT_SHARED,
    DEREF_COERCION_DEPTH, GLOBAL_CALL_DEPTH, NORMALIZE_DEPTH,
    AST_TO_TYPE_DEPTH, TYPE_RESOLUTION_STACK, NORMALIZE_TYPE_STACK,
    MAX_INFERENCE_DEPTH,
    read_param_classification, is_stdlib_toplevel_path,
    collect_inline_mount_reexports_recursive, has_declassify_attr_on_function,
    collect_named_types_from_item, parse_descriptor_type_string,
    register_variant_signature_for_lazy,
};
#[allow(unused_imports)]
use std::cell::Cell;
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
    // ========================================================================
    // #91 Phase 3 — pre-resolved-static-call side-table API.
    //
    // The typechecker writes to the side-table during inference at every
    // method-call resolution success point.  After type checking
    // completes, `commit_resolved_call_targets` drains the table into
    // the AST so the VBC codegen's fast path picks up the resolutions.
    //
    // Architectural rationale lives on the field's docstring at
    // `TypeChecker::resolved_call_targets` (infer/mod.rs).
    // ========================================================================

    /// Record that the method-call expression at `span` resolves to
    /// the function with canonical name `qualified_name`.  Called by
    /// `infer_method_call_inner_impl` at every static-method dispatch
    /// success point.
    pub(crate) fn record_resolved_static_call(
        &mut self,
        span: verum_ast::span::Span,
        qualified_name: impl Into<verum_common::Text>,
    ) {
        self.resolved_call_targets.insert(
            span,
            verum_ast::expr::ResolvedCallTarget::StaticCall {
                qualified_name: qualified_name.into(),
            },
        );
    }

    /// Record that the method-call expression at `span` resolves to
    /// a variant constructor with the given tag and (optionally)
    /// parent-type canonical name.
    pub(crate) fn record_resolved_variant_ctor(
        &mut self,
        span: verum_ast::span::Span,
        tag: u32,
        parent_type_name: Option<verum_common::Text>,
    ) {
        self.resolved_call_targets.insert(
            span,
            verum_ast::expr::ResolvedCallTarget::VariantCtor {
                tag,
                parent_type_name,
            },
        );
    }

    /// Walk `module` mutably and stamp every `MethodCall` expression's
    /// `resolved_call_target` from the typechecker's side-table.
    /// Idempotent — running twice produces the same AST.
    ///
    /// Call this AFTER `phase_type_check` but BEFORE the codegen
    /// phase so the codegen's `compile_method_call` fast path sees
    /// the resolutions.
    pub fn commit_resolved_call_targets(&self, module: &mut verum_ast::Module) {
        if self.resolved_call_targets.is_empty() {
            return;
        }
        for item in module.items.iter_mut() {
            commit_resolved_in_item(item, &self.resolved_call_targets);
        }
    }

    /// Take ownership of the resolved-call-target side-table,
    /// leaving an empty table behind.  Used by the pipeline to
    /// transfer the table from the dropped typechecker to the
    /// pipeline state, where it can be applied to the AST via
    /// `apply_resolved_call_targets` (the call site has `&mut Module`
    /// access; the typechecker phase has only `&Module`).
    pub fn take_resolved_call_targets(
        &mut self,
    ) -> std::collections::HashMap<
        verum_ast::span::Span,
        verum_ast::expr::ResolvedCallTarget,
    > {
        std::mem::take(&mut self.resolved_call_targets)
    }
}

/// Walk `module` mutably and stamp every `MethodCall` expression's
/// `resolved_call_target` from the supplied side-table.  Same shape
/// as `TypeChecker::commit_resolved_call_targets`, exposed as a
/// free function so the pipeline can apply the table without
/// holding a TypeChecker.
///
/// Idempotent — running twice produces the same AST.
/// No-op when `table` is empty.
pub fn apply_resolved_call_targets(
    module: &mut verum_ast::Module,
    table: &std::collections::HashMap<
        verum_ast::span::Span,
        verum_ast::expr::ResolvedCallTarget,
    >,
) {
    if table.is_empty() {
        return;
    }
    for item in module.items.iter_mut() {
        commit_resolved_in_item(item, table);
    }
}

fn commit_resolved_in_item(
    item: &mut verum_ast::decl::Item,
    table: &std::collections::HashMap<
        verum_ast::span::Span,
        verum_ast::expr::ResolvedCallTarget,
    >,
) {
    use verum_ast::decl::ItemKind;
    use verum_common::Maybe;
    match &mut item.kind {
        ItemKind::Function(func) => {
            if let Maybe::Some(body) = &mut func.body {
                commit_resolved_in_function_body(body, table);
            }
        }
        ItemKind::Impl(impl_decl) => {
            for impl_item in impl_decl.items.iter_mut() {
                if let verum_ast::decl::ImplItemKind::Function(func) = &mut impl_item.kind
                    && let Maybe::Some(body) = &mut func.body
                {
                    commit_resolved_in_function_body(body, table);
                }
            }
        }
        ItemKind::Const(decl) => commit_resolved_in_expr(&mut decl.value, table),
        ItemKind::Static(decl) => commit_resolved_in_expr(&mut decl.value, table),
        _ => {}
    }
}

fn commit_resolved_in_function_body(
    body: &mut verum_ast::decl::FunctionBody,
    table: &std::collections::HashMap<
        verum_ast::span::Span,
        verum_ast::expr::ResolvedCallTarget,
    >,
) {
    use verum_ast::decl::FunctionBody;
    match body {
        FunctionBody::Block(block) => commit_resolved_in_block(block, table),
        FunctionBody::Expr(expr) => commit_resolved_in_expr(expr, table),
    }
}

fn commit_resolved_in_block(
    block: &mut verum_ast::expr::Block,
    table: &std::collections::HashMap<
        verum_ast::span::Span,
        verum_ast::expr::ResolvedCallTarget,
    >,
) {
    use verum_common::Maybe;
    for stmt in block.stmts.iter_mut() {
        commit_resolved_in_stmt(stmt, table);
    }
    if let Maybe::Some(tail) = &mut block.expr {
        commit_resolved_in_expr(tail, table);
    }
}

fn commit_resolved_in_stmt(
    stmt: &mut verum_ast::Stmt,
    table: &std::collections::HashMap<
        verum_ast::span::Span,
        verum_ast::expr::ResolvedCallTarget,
    >,
) {
    use verum_ast::stmt::StmtKind;
    use verum_common::Maybe;
    match &mut stmt.kind {
        StmtKind::Let { value, .. } => {
            if let Maybe::Some(v) = value {
                commit_resolved_in_expr(v, table);
            }
        }
        StmtKind::LetElse {
            value, else_block, ..
        } => {
            commit_resolved_in_expr(value, table);
            commit_resolved_in_block(else_block, table);
        }
        StmtKind::Expr { expr, .. } => commit_resolved_in_expr(expr, table),
        StmtKind::Item(item) => commit_resolved_in_item(item, table),
        StmtKind::Defer(e) | StmtKind::Errdefer(e) => commit_resolved_in_expr(e, table),
        StmtKind::Provide { value, .. } => commit_resolved_in_expr(value, table),
        StmtKind::ProvideScope { value, block, .. } => {
            commit_resolved_in_expr(value, table);
            commit_resolved_in_expr(block, table);
        }
        _ => {}
    }
}

fn commit_resolved_in_expr(
    expr: &mut verum_ast::Expr,
    table: &std::collections::HashMap<
        verum_ast::span::Span,
        verum_ast::expr::ResolvedCallTarget,
    >,
) {
    use verum_ast::expr::ExprKind;
    use verum_common::Maybe;
    // Stamp THIS node first if the table has a resolution for its span.
    if matches!(expr.kind, ExprKind::MethodCall { .. })
        && let Some(target) = table.get(&expr.span)
    {
        expr.resolved_call_target = Some(target.clone());
    }
    // Then recurse into children so nested method calls also get stamped.
    match &mut expr.kind {
        ExprKind::MethodCall {
            receiver, args, ..
        } => {
            commit_resolved_in_expr(receiver, table);
            for a in args.iter_mut() {
                commit_resolved_in_expr(a, table);
            }
        }
        ExprKind::Call { func, args, .. } => {
            commit_resolved_in_expr(func, table);
            for a in args.iter_mut() {
                commit_resolved_in_expr(a, table);
            }
        }
        ExprKind::Binary { left, right, .. } => {
            commit_resolved_in_expr(left, table);
            commit_resolved_in_expr(right, table);
        }
        ExprKind::Unary { expr, .. } => commit_resolved_in_expr(expr, table),
        ExprKind::Field { expr, .. }
        | ExprKind::OptionalChain { expr, .. }
        | ExprKind::TupleIndex { expr, .. } => commit_resolved_in_expr(expr, table),
        ExprKind::Index { expr, index } => {
            commit_resolved_in_expr(expr, table);
            commit_resolved_in_expr(index, table);
        }
        ExprKind::Pipeline { left, right }
        | ExprKind::NullCoalesce { left, right } => {
            commit_resolved_in_expr(left, table);
            commit_resolved_in_expr(right, table);
        }
        ExprKind::Cast { expr, .. } => commit_resolved_in_expr(expr, table),
        ExprKind::Try(e) | ExprKind::TryBlock(e) => commit_resolved_in_expr(e, table),
        ExprKind::Block(block) => commit_resolved_in_block(block, table),
        ExprKind::If {
            then_branch,
            else_branch,
            condition,
            ..
        } => {
            // IfCondition is a SmallVec<ConditionKind> where each
            // ConditionKind is either a bare Expr or a let-binding
            // (`let pattern = value`). Recurse into each contained
            // expression so nested method calls inside `if let`
            // chains and bool conditions get stamped too.
            for ck in condition.conditions.iter_mut() {
                match ck {
                    verum_ast::expr::ConditionKind::Expr(e) => {
                        commit_resolved_in_expr(e, table)
                    }
                    verum_ast::expr::ConditionKind::Let { value, .. } => {
                        commit_resolved_in_expr(value, table)
                    }
                }
            }
            commit_resolved_in_block(then_branch, table);
            if let Maybe::Some(eb) = else_branch {
                commit_resolved_in_expr(eb, table);
            }
        }
        ExprKind::Match { expr, arms } => {
            commit_resolved_in_expr(expr, table);
            for arm in arms.iter_mut() {
                if let Maybe::Some(g) = &mut arm.guard {
                    commit_resolved_in_expr(g, table);
                }
                commit_resolved_in_expr(&mut arm.body, table);
            }
        }
        ExprKind::Loop { body, .. } => commit_resolved_in_block(body, table),
        ExprKind::While { condition, body, .. } => {
            commit_resolved_in_expr(condition, table);
            commit_resolved_in_block(body, table);
        }
        ExprKind::For { iter, body, .. } => {
            commit_resolved_in_expr(iter, table);
            commit_resolved_in_block(body, table);
        }
        ExprKind::Closure { body, .. } => commit_resolved_in_expr(body, table),
        ExprKind::Return(e) => {
            if let Maybe::Some(e) = e {
                commit_resolved_in_expr(e, table);
            }
        }
        ExprKind::Async(block) => commit_resolved_in_block(block, table),
        ExprKind::Await(inner) => commit_resolved_in_expr(inner, table),
        ExprKind::Paren(inner) => commit_resolved_in_expr(inner, table),
        ExprKind::Throw(error) => commit_resolved_in_expr(error, table),
        ExprKind::Range { start, end, .. } => {
            if let Maybe::Some(s) = start {
                commit_resolved_in_expr(s, table);
            }
            if let Maybe::Some(e) = end {
                commit_resolved_in_expr(e, table);
            }
        }
        // Leaves and structurally simple kinds: nothing to recurse.
        _ => {}
    }
}

impl TypeChecker {
    /// QTT validation: walk a function body and confirm every
    /// declared binding's runtime usage matches its declared
    /// quantity. Returns `Ok(usage_map)` on success or the first
    /// `QttViolation` encountered (alphabetically first, for
    /// deterministic diagnostics).
    ///

    /// This is the integration entry point: callers (codegen,
    /// LSP, `@verify(formal)` boundary) supply the per-binding
    /// declared quantities (most often derived from explicit
    /// `meta` / `&checked` annotations or function-signature
    /// `Quantity` fields) plus the body, and the checker validates
    /// them against the QTT calculus.
    pub fn check_function_qtt(
        &self,
        declarations: &std::collections::HashMap<Text, crate::ty::Quantity>,
        body: &verum_ast::expr::Expr,
    ) -> std::result::Result<crate::qtt_usage::UsageMap, crate::qtt_usage::QttViolation> {
        let tracked: std::collections::HashSet<Text> = declarations.keys().cloned().collect();
        let usage = crate::qtt_walker::walk_expr(&tracked, body);
        crate::qtt_usage::check_usage(declarations, &usage)?;
        Ok(usage)
    }

    /// Increment the unified inference depth counter and return an RAII guard.
    /// The guard decrements the counter on drop, preventing stuck counters
    /// after panics or early returns.
    #[inline]
    pub(super) fn inc_inference_depth(&self, context: &str) -> Result<InferenceDepthGuard> {
        let depth = self.inference_depth.get() + 1;
        self.inference_depth.set(depth);
        if depth > MAX_INFERENCE_DEPTH {
            self.inference_depth.set(depth - 1);
            return Err(TypeError::Other(Text::from(format!(
                "type inference recursion limit exceeded in {} (depth {})",
                context, depth
            ))));
        }
        Ok(InferenceDepthGuard {
            depth_cell: &self.inference_depth as *const Cell<u32>,
        })
    }

    /// Decrement the unified inference depth counter.
    /// Kept for any remaining manual usage, but prefer the RAII guard from inc_inference_depth.
    #[inline]
    pub(super) fn dec_inference_depth(&self) {
        let d = self.inference_depth.get();
        if d > 0 {
            self.inference_depth.set(d - 1);
        }
    }

    /// Create a TypeChecker with language primitives only.
    ///

    /// STDLIB-AGNOSTIC: This constructor does NOT include hardcoded stdlib types.
    /// Stdlib types (Maybe, Result, List, etc.) are loaded dynamically from:
    /// - core/*.vr source files (via pipeline.load_stdlib_modules())
    /// - Pre-compiled VBC archives (via CoreMetadata)
    ///

    /// Only language primitives (Bool, Unit) and compiler intrinsics are included.
    pub fn new() -> Self {
        Self {
            ctx: TypeContext::new(),
            unifier: Unifier::new(),
            refinement: RefinementChecker::new(Default::default()),
            subtyping: Subtyping::new(),
            const_eval: ConstEvaluator::new(),
            protocol_checker: Shared::new(parking_lot::RwLock::new(ProtocolChecker::new())),
            integer_hierarchy: IntegerHierarchy::new(),
            // STDLIB-AGNOSTIC: Use new_minimal() instead of new() to avoid hardcoded types.
            // Type constructors are registered when stdlib is loaded from source files.
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
            resolved_call_targets: std::collections::HashMap::new(),
        }
    }

    /// Create a new type checker with stdlib metadata loaded from stdlib.vbc.
    ///

    /// This is the PREFERRED constructor for compiling user code.
    /// Types and methods are loaded from stdlib.vbca metadata, enabling
    /// type checking of stdlib types without parsing .vr source files.
    pub fn new_with_core(
        metadata: std::sync::Arc<crate::core_metadata::CoreMetadata>,
    ) -> Self {
        // T2-extended-perf: defer eager `load_stdlib_from_metadata`
        // (~3.8s on release cold start, ~5.3s debug).  The
        // pipeline's `phase_type_check` calls
        // `register_stdlib_types_for_module(user_module)` which
        // pre-loads ONLY the stdlib types the user code actually
        // references (~10s of types vs 1000+).  Drops cold-start
        // typecheck from 3.8s → ~50ms for a hello.vr-style script.
        //
        // Production callers MUST drive `register_stdlib_types_for_module`
        // before the main typecheck pass.  Audit/corpus tooling
        // that needs the entire stdlib pre-registered upfront uses
        // [`new_with_core_eager`](Self::new_with_core_eager) instead.
        //
        // Takes `Arc<CoreMetadata>` so production callers (which
        // hold an `Arc` from the pipeline's lazy embedded sidecar)
        // hand off the metadata in O(1) — no 15ms 3MB deep clone.
        let mut checker = Self::with_minimal_context();
        checker.register_coercion_markers_from_metadata(&metadata);
        // Eagerly register `<Type>.<method>` static fns into env so
        // multi-mount field-access paths (`Map.new()` when both
        // `mount core.collections.map.Map` and a sibling collection
        // mount put `Map` into module_aliases instead of env) resolve
        // via env.lookup("Map.new").  Sister call to
        // `register_stdlib_consts_from_metadata` — both are bounded by
        // the metadata size and run in single-millisecond range.
        checker.register_stdlib_static_methods_from_metadata(&metadata);
        checker.core_metadata = Maybe::Some(metadata);
        checker
    }

    /// One-shot scan of `metadata.implementations` that mirrors
    /// `verum_compiler::stdlib_coercion_registry::scan_protocol_implementations`
    /// — but for the USER-SIDE typecheck path that doesn't re-walk
    /// stdlib AST.  Without this, the unifier's coercion-marker
    /// registries (ArrayCoercible / IntCoercible / TensorLike /
    /// Indexable / RangeLike / BytewiseFfi / SizedNumeric) stay
    /// empty in `verum run` / `verum build`, and every
    /// `let bs: List<Byte> = [1, 2, 3]` style coercion fails with
    /// `expected 'List<Byte>', found '[Byte; 3]'` because
    /// `is_array_coercible("List")` returns false.
    ///
    /// Closes the user-side gap left by the stdlib-bootstrap-only
    /// scan path: the bootstrap populates the unifier built INSIDE
    /// the stdlib precompile process, but `verum run` constructs a
    /// fresh unifier and the registries default to empty.
    ///
    /// Idempotent — every `register_*` is a HashSet insert.
    fn register_coercion_markers_from_metadata(
        &mut self,
        metadata: &crate::core_metadata::CoreMetadata,
    ) {
        for impl_desc in metadata.implementations.iter() {
            let proto = impl_desc.protocol.as_str();
            let target = impl_desc.target_type.clone();
            if target.as_str().is_empty() {
                continue;
            }
            match proto {
                "IntCoercible" => self.unifier.register_int_coercible_type(target),
                "TensorLike" => self.unifier.register_tensor_family_type(target),
                "Indexable" => self.unifier.register_indexable_type(target),
                "RangeLike" => self.unifier.register_range_like_type(target),
                "BytewiseFfi" => self.unifier.register_bytewise_ffi_type(target),
                "SizedNumeric" => self.unifier.register_sized_numeric_type(target),
                "ArrayCoercible" => {
                    self.unifier.register_array_coercible_type(target.clone());
                    crate::subtype::register_global_array_coercible(target);
                }
                _ => {}
            }
        }
    }

    /// Hand stdlib metadata to a TypeChecker constructed via a
    /// non-`new_with_core` path (e.g.
    /// [`with_shared_methods`](Self::with_shared_methods) or
    /// [`with_minimal_context`](Self::with_minimal_context)).
    ///
    /// Required so the receiver-driven lazy stdlib-type loader
    /// (`infer_method_call_inner_impl`'s `ensure_stdlib_type_loaded`
    /// call on the receiver's type-name) actually has a metadata
    /// table to pull from.  Without it, every method call on a
    /// stdlib type that wasn't named explicitly by user code (e.g.
    /// `pool.acquire().await?` returning `AsyncPgPoolGuard`
    /// inferred indirectly through `Result.Ok` arm) fails the
    /// inherent-method bucket lookup and surfaces as
    /// `MethodNotFound` despite the bodies being in the
    /// precompiled archive.
    pub fn set_core_metadata(
        &mut self,
        metadata: std::sync::Arc<crate::core_metadata::CoreMetadata>,
    ) {
        self.register_coercion_markers_from_metadata(&metadata);
        self.core_metadata = Maybe::Some(metadata);
    }

    /// Eager construction — registers every type/protocol/function
    /// from the supplied metadata upfront.  ~3.8s on release cold
    /// start.  Used by audit / corpus tooling that needs the
    /// entire stdlib pre-registered regardless of what user code
    /// references.  Production `verum run` / `verum build` use the
    /// lazy [`new_with_core`](Self::new_with_core) path instead.
    pub fn new_with_core_eager(
        metadata: std::sync::Arc<crate::core_metadata::CoreMetadata>,
    ) -> Self {
        let mut checker = Self::with_minimal_context();
        checker.load_stdlib_from_metadata(&metadata);
        checker.core_metadata = Maybe::Some(metadata);
        checker
    }

    /// Pre-scan a user module AST for every named type / protocol /
    /// generic-type-arg, and register each from `core_metadata` if
    /// not yet loaded.  Pairs with [`Self::new_with_core_lazy`] so a
    /// single pre-pass covers the entire typecheck of user code.
    ///
    /// O(names_in_module) — typically tens to low hundreds for a
    /// real script, vs O(stdlib_total) ≈ 1000+ for the eager path.
    pub fn register_stdlib_types_for_module(&mut self, module: &verum_ast::Module) {
        if matches!(self.core_metadata, Maybe::None) {
            return;
        }
        let mut needed: std::collections::HashSet<Text> =
            std::collections::HashSet::new();
        for item in module.items.iter() {
            collect_named_types_from_item(item, &mut needed);
        }
        // Drain the set into the registration loop.  Each `ensure`
        // call may register additional dependencies (variant payload
        // types, field types, super-protocols) that were transitively
        // referenced — those are picked up by repeated passes until
        // the set stabilises.
        let mut to_load: Vec<Text> = needed.into_iter().collect();
        let mut already: std::collections::HashSet<Text> =
            std::collections::HashSet::new();
        while let Some(name) = to_load.pop() {
            if !already.insert(name.clone()) {
                continue;
            }
            self.ensure_stdlib_type_loaded(&name, &mut to_load);
        }

        // Register inductive constructors for pattern matching.
        // The eager `load_stdlib_from_metadata` calls this at the end
        // of its single-pass walk; the lazy path needs the same hook
        // so `match res { Ok(p) => …, Err(e) => … }` patterns
        // resolve against Result's variant body, and `Ok(x)` /
        // `Err(e)` / `Some(x)` value-position uses bind to the
        // constructor.  Without this, every pattern match on
        // Result/Maybe/IoResult fails `Pattern expects a variant
        // type, but scrutinee has type Result/IoResult`.
        let metadata = match &self.core_metadata {
            Maybe::Some(m) => m.clone(),
            Maybe::None => return,
        };
        self.register_stdlib_constructors_from_metadata(&metadata);
        // #97 — register stdlib consts (zero-arg fns with is_const=true)
        // as values in the env so user code's bare references
        // (`let s = SSO_CAPACITY;`) resolve via env.lookup.  Cheap to
        // do eagerly; ~3000 inserts run in sub-millisecond.
        self.register_stdlib_consts_from_metadata(&metadata);
        // Register stdlib `<Type>.<method>` static fns under bare
        // canonical form so `Map.new` / `List.new` / etc. resolve
        // through env.lookup at the field-access fallback in
        // expr.rs::infer_expr_field (line ~6618).  Closes the
        // multi-mount typechecker/codegen registry-key drift
        // documented in MEMORY.md (session 2026-05-12 wrap).
        self.register_stdlib_static_methods_from_metadata(&metadata);

        // Unconditionally register every type alias from metadata
        // into the unifier's alias registry.  Aliases are cheap
        // (single TypeRef payload + param-name list) and a user
        // script may reference an alias indirectly through a
        // function's return type without ever naming the alias
        // directly — `fs_metadata(p) -> IoResult<Metadata>` returns
        // IoResult but the user code never writes "IoResult".  The
        // alias needs to be in the registry by the time pattern
        // matching expands the type.
        for (alias_name, td) in metadata.types.iter() {
            if let crate::core_metadata::TypeDescriptorKind::Alias { target } = &td.kind {
                let target_ty = parse_descriptor_type_string(target.as_str());
                self.ctx
                    .define_alias(alias_name.clone(), target_ty.clone());
                self.unifier
                    .register_type_alias(alias_name.clone(), target_ty);
                let param_names: List<Text> = td
                    .generic_params
                    .iter()
                    .map(|gp| gp.name.clone())
                    .collect();
                if !param_names.is_empty() {
                    self.unifier
                        .register_type_alias_params(alias_name.clone(), param_names);
                }
            }
        }
    }

    /// Register a single stdlib type from `core_metadata` if it's
    /// declared there and not yet registered.  Pushes any
    /// transitively-referenced type names into `pending` so the
    /// caller's loop can continue until the closure stabilises.
    /// Idempotent under repeated calls (already-loaded types
    /// short-circuit on `ctx.lookup_type`).
    pub fn ensure_stdlib_type_loaded(
        &mut self,
        name: &Text,
        pending: &mut Vec<Text>,
    ) {
        let metadata = match &self.core_metadata {
            Maybe::Some(m) => m.clone(),
            Maybe::None => return,
        };
        let type_desc = match metadata.types.get(name) {
            Some(td) => td.clone(),
            None => return,
        };
        // For built-in primitive types (Text, Int, Float, Bool, …)
        // already registered by `register_builtins`, the type
        // definition does NOT need re-registration — but its
        // inherent-method bucket DOES need population from
        // `core_metadata`, otherwise method-call typecheck
        // (`text.push_str(...)`, `text.trim()`, …) fails despite
        // the bodies being present in the precompiled archive.
        // Run the type-definition registration only when the type
        // isn't yet in ctx, but ALWAYS run inherent-method
        // registration (which is idempotent — skips already-
        // populated method names).
        if self.ctx.lookup_type(name.as_str()).is_none() {
            // Convert + register this single type.  Mirror of the body
            // of `load_stdlib_from_metadata` reduced to one entry.
            let ty = self.type_descriptor_to_type(&type_desc);
            self.ctx.define_type(name.clone(), ty.clone());
            self.ctx.env.insert(name.clone(), TypeScheme::mono(ty.clone()));

            // Mirror the eager loader's `type_generics_count` write.
            // Without this, `try_resolve_variant_constructor` falls
            // into its `generics_count == 0` branch and synthesises
            // bare `Type::Named { path: parent, args: [] }` for the
            // constructor return — every `Ok(x)` / `Err(e)` /
            // `Some(x)` value-position use against an expected
            // `Result<T, E>` then fails unification with "found
            // 'Result'" because the synthesised type carries no
            // type arguments.  The eager loader ran this at
            // `load_stdlib_from_metadata` (line 1981-1984); the
            // lazy `ensure_stdlib_type_loaded` path missed it.
            if !type_desc.generic_params.is_empty() {
                self.type_generics_count
                    .insert(name.clone(), type_desc.generic_params.len());
            }

            // Type alias: also register in BOTH the ctx's alias
            // registry AND the unifier's alias registry so
            // `try_expand_alias` and the pattern matcher can resolve
            // `IoResult<T>` → `Result<T, IoError>` etc.  Generic
            // alias param-name list goes through
            // `register_type_alias_params` so positional type
            // argument substitution works (`IoResult<Text>` →
            // `Result<Text, StreamError>`, not just
            // `Result<T, StreamError>`).
            //
            // Pre-fix every match pattern over a stdlib alias
            // (`match res { Ok(p) => …, Err(e) => … }` on
            // `IoResult<Text>`) failed `Pattern expects a variant
            // type, but scrutinee has type IoResult<...>`.
            if let crate::core_metadata::TypeDescriptorKind::Alias { target } =
                &type_desc.kind
            {
                // Parse the target via `parse_descriptor_type_string`
                // so generic-instantiated aliases (`Bytes = List<Byte>`,
                // `IoResult<T> = Result<T, IoError>`) produce the right
                // structural shape — `Type::Generic { name, args }` —
                // instead of a malformed `Type::Named { path:
                // "List<Byte>", args: [] }`.  Pre-fix the unifier's
                // `resolve_aliased_head_name` couldn't walk the Generic
                // head of these aliases (Bytes never resolved to "List"),
                // so the alias-aware method-bucket lookup at
                // `infer_method_call_inner_impl` missed.  Mirrors the
                // eager loader's parse path at line ~696.
                let target_ty = parse_descriptor_type_string(target.as_str());
                self.ctx.define_alias(name.clone(), target_ty.clone());
                self.unifier
                    .register_type_alias(name.clone(), target_ty);
                let param_names: List<Text> = type_desc
                    .generic_params
                    .iter()
                    .map(|gp| gp.name.clone())
                    .collect();
                if !param_names.is_empty() {
                    self.unifier
                        .register_type_alias_params(name.clone(), param_names);
                }
                pending.push(target.clone());
            }

            // Variant signatures — same logic as the eager loader, gated
            // to this single type.  Pushes payload type names into
            // `pending` so the loop registers them too.
            if let crate::core_metadata::TypeDescriptorKind::Variant { cases } = &type_desc.kind {
                register_variant_signature_for_lazy(self, name, &type_desc, cases, pending);
            }
            // Record fields — push field type names into `pending`.
            if let crate::core_metadata::TypeDescriptorKind::Record { fields } = &type_desc.kind {
                for f in fields.iter() {
                    if !f.ty.is_empty() {
                        pending.push(f.ty.clone());
                    }
                }
            }
            // Generic param defaults — also dependencies.
            for gp in type_desc.generic_params.iter() {
                if let Maybe::Some(default_text) = &gp.default {
                    pending.push(default_text.clone());
                }
            }
        }

        // FUNDAMENTAL #3 — transparent-wrapper newtype inner type
        // registration.  Mirror of the source-decl path at
        // `infer/decls.rs:454` so archive-loaded `type X is T;` and
        // `type X is (T);` produce a `__newtype_inner_X` key — the
        // same one `infer/env.rs:1993-2014` consults when typechecking
        // `<wrapper>.0` access.
        //
        // ALWAYS runs (outside the `lookup_type(name).is_none()` gate
        // above) — mirrors the inherent-methods discipline.  Without
        // this, types already registered into ctx by an upstream
        // pathway (`archive_ctx_loader::populate_ctx_from_archive`
        // primary types, eager `load_stdlib_from_metadata`, or a
        // prior `ensure_stdlib_type_loaded` call) miss the
        // `__newtype_inner_X` registration.  Idempotent: re-defining
        // the same key with the same value is a no-op.
        //
        // The inner-type field is keyed as "_0" by
        // `compile_type_decl`'s Newtype / Tuple-single arms; the
        // parser produces only one such field per transparent
        // wrapper.  Use that field's `ty` text and route it through
        // `parse_descriptor_type_string` so generic-instantiated
        // inner types (`type Cell<T> is (T)`) resolve to the right
        // structural shape.
        if type_desc.is_transparent_wrapper
            && let crate::core_metadata::TypeDescriptorKind::Record { fields } = &type_desc.kind
            && let Some(first_field) = fields.first()
            && !first_field.ty.is_empty()
        {
            let inner_key_text =
                verum_common::Text::from(format!("__newtype_inner_{}", name).as_str());
            if self.ctx.lookup_type(inner_key_text.as_str()).is_none() {
                let inner_ty = parse_descriptor_type_string(first_field.ty.as_str());
                self.ctx.define_type(inner_key_text, inner_ty);
            }
        }

        // ALWAYS register inherent methods, even when the type
        // itself was already in ctx (e.g. primitives like Text,
        // List, Map registered via `register_builtins`).  Without
        // this unconditional pass, every `text.push_str(...)` /
        // `list.iter()` / `map.get()` call site fails `no method
        // named …` typecheck despite the bodies being in the
        // precompiled archive.  The pass is idempotent — skips
        // method names already populated in the
        // inherent_methods bucket.
        //
        // Method signatures push their referenced type names into
        // `pending` so dependent stdlib types (Path, Metadata,
        // IoResult, …) load transitively — needed for alias
        // resolution + variant pattern matching at user-code
        // call sites.
        let referenced = self.register_inherent_methods_from_metadata(name, &type_desc, &metadata);
        for r in referenced {
            pending.push(r);
        }

        // #130 — When the loaded type is itself a Protocol, register
        // its body (methods / super-protocols / type-params) into
        // `protocol_checker` so impl-registration consumers can look
        // up its method signatures.  The eager
        // `load_stdlib_from_metadata` registers every protocol
        // upfront via `metadata.protocols`; the lazy path needs the
        // same hook gated to this single name.
        if matches!(
            type_desc.kind,
            crate::core_metadata::TypeDescriptorKind::Protocol { .. }
        ) {
            self.register_stdlib_protocol_for_name(name, &metadata);
        }

        // #130 — register protocol implementations that target this
        // type.  Pre-fix the lazy loader registered the type
        // definition + inherent methods but NOT the protocol impls
        // recorded in `metadata.implementations`, so
        // `protocol_checker.get_implementations(IntoList<_>)` returned
        // empty — the canonical `xs.into_iter().map(f).collect()`
        // chain failed at type-check because the dispatcher had no
        // impl to walk for `IntoList<_>`.
        //
        // Pairs with the archive_metadata fix that populates
        // `ImplementationDescriptor.protocol` from the VBC type
        // table's `ProtocolId` field (was hardcoded `Text::default()`).
        //
        // Stdlib-agnostic per `crates/verum_types/src/CLAUDE.md`:
        // the impl list comes from `metadata.implementations`, not a
        // hardcoded mapping.  Adding `Foldable` / `Functor` / etc.
        // implementations to a stdlib type works identically.
        let proto_deps = self.register_stdlib_impls_for_target(name, &metadata);
        for proto_name in proto_deps {
            pending.push(proto_name);
        }
    }

    /// Register the body of a single stdlib protocol from
    /// `metadata.protocols[name]` into `protocol_checker`.  Idempotent
    /// (no-op if `protocol_checker.get_protocol(name)` already returns
    /// `Some`).  Mirror of the eager loop in
    /// `load_stdlib_from_metadata` lines ~2178-2275, gated to a
    /// single name for the lazy path.
    pub(super) fn register_stdlib_protocol_for_name(
        &mut self,
        name: &Text,
        metadata: &crate::core_metadata::CoreMetadata,
    ) {
        use crate::protocol::{Protocol, ProtocolMethod};
        let protocol_desc = match metadata.protocols.get(name) {
            Some(d) => d,
            None => return,
        };

        // Conservative MERGE policy with `register_standard_protocols`
        // hardcoded baseline (in `crates/verum_types/src/protocol.rs:1973+`):
        //
        //  * If the existing protocol has ≥2 methods, it's a
        //    well-formed hardcoded entry (Eq/Ord/Show/PartialOrd/etc
        //    all seed multiple methods with hand-curated TypeVar
        //    shapes that round-trip correctly through the unifier).
        //    OVERWRITING those with VBCA-derived signatures breaks
        //    operator-protocol unification (Layer E gap: TypeRef::Generic
        //    method-local param ids render as concrete TypeIds via
        //    fallback to PTR / well-known-type-name — so e.g. PartialOrd's
        //    `lt(T, T) -> Bool` gets stomped by `lt(Heap<_,_>, Heap<_,_>)
        //    -> Bool`).  Skip in this case — keep the hardcoded entry.
        //  * If the existing protocol has 0 or 1 methods, it's a
        //    stub (e.g. Iterator hardcoded with only `next`).  Layer D
        //    populates the missing 73 default-method signatures.
        //    SUPPLEMENT in this case — preserve the hardcoded
        //    protocol's `associated_types` + `associated_consts`
        //    (the stub seeds `Iterator::Item`, etc.) but replace the
        //    methods table with the VBCA-derived 74 entries.
        //
        // This split is the cleanest co-existence with the legacy
        // hardcoded path until `register_standard_protocols` is
        // removed in favour of pure metadata-driven registration
        // (separate refactor; CLAUDE.md "no stdlib knowledge in
        // compiler" rule).
        let (existing_assoc_types, existing_assoc_consts, existing_method_count) = {
            let pc = self.protocol_checker.read();
            match pc.get_protocol(name) {
                Maybe::Some(p) => (
                    p.associated_types.clone(),
                    p.associated_consts.clone(),
                    p.methods.len(),
                ),
                Maybe::None => (verum_common::Map::new(), verum_common::Map::new(), 0),
            }
        };
        // Skip when the hardcoded baseline is well-formed (≥2 methods).
        if existing_method_count >= 2 {
            return;
        }

        // Convert each metadata-derived parameter / return type
        // string back into a structured `Type`.  The
        // descriptor-string parser handles primitives, bare names,
        // generic instantiations (`Maybe<T>`), references (`&T`),
        // function types, and the `__opaque_type_N` /
        // `__generic_N` placeholders (mapped to fresh TypeVars).
        // After #131 Layer E landed, codegen emits proper
        // `TypeRef::Generic` for protocol-level params, method-
        // local params, AND associated-type projections
        // (`Self.Item`) — archive_metadata renders the unresolved
        // ones as `__generic_N` placeholders that the parser
        // converts to fresh TypeVars.  No additional safety guard
        // needed at this site.
        let to_type = |s: &Text| -> Type { parse_descriptor_type_string(s.as_str()) };
        let mut methods = verum_common::Map::new();
        for m in protocol_desc.required_methods.iter() {
            let params: List<Type> = m.params.iter().map(|p| to_type(&p.ty)).collect();
            let return_type = to_type(&m.return_type);
            let method_type = Type::function(params, return_type);
            let protocol_method =
                ProtocolMethod::simple(m.name.clone(), method_type, false);
            methods.insert(m.name.clone(), protocol_method);
        }
        for m in protocol_desc.default_methods.iter() {
            let params: List<Type> = m.params.iter().map(|p| to_type(&p.ty)).collect();
            let return_type = to_type(&m.return_type);
            let method_type = Type::function(params, return_type);
            let protocol_method =
                ProtocolMethod::simple(m.name.clone(), method_type, true);
            methods.insert(m.name.clone(), protocol_method);
        }

        let protocol = Protocol {
            name: name.clone(),
            kind: crate::protocol::ProtocolKind::Constraint,
            type_params: protocol_desc
                .generic_params
                .iter()
                .map(|g| crate::protocol::TypeParam {
                    name: g.name.clone(),
                    bounds: g
                        .bounds
                        .iter()
                        .map(|b| crate::protocol::ProtocolBound {
                            protocol: Self::text_to_path(b),
                            args: List::new(),
                            is_negative: false,
                        })
                        .collect(),
                    default: g.default.as_ref().map(|d| Type::Named {
                        path: Self::text_to_path(d),
                        args: List::new(),
                    }),
                })
                .collect(),
            methods,
            associated_types: existing_assoc_types,
            associated_consts: existing_assoc_consts,
            super_protocols: protocol_desc
                .super_protocols
                .iter()
                .map(|sp| crate::protocol::ProtocolBound {
                    protocol: Self::text_to_path(sp),
                    args: List::new(),
                    is_negative: false,
                })
                .collect(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::None,
            span: Span::default(),
        };
        let _ = self.protocol_checker.write().register_protocol(protocol);
    }

    /// Register every protocol implementation in
    /// `metadata.implementations` that targets `type_name`.  Returns
    /// the set of protocol names referenced by these impls so the
    /// caller can push them onto the pending queue (so the protocol's
    /// own type-definition + body load before any subsequent
    /// dispatch tries to resolve a default method on the impl).
    /// Mirror of the eager loop in `load_stdlib_from_metadata` lines
    /// ~2278-2320, gated to a single target.
    pub(super) fn register_stdlib_impls_for_target(
        &mut self,
        type_name: &Text,
        metadata: &crate::core_metadata::CoreMetadata,
    ) -> Vec<Text> {
        use crate::protocol::ProtocolImpl;
        let mut proto_deps: Vec<Text> = Vec::new();
        for impl_desc in metadata.implementations.iter() {
            if impl_desc.target_type.as_str() != type_name.as_str() {
                continue;
            }
            if impl_desc.protocol.as_str().is_empty() {
                continue;
            }
            // Make sure the protocol body is registered so we can
            // pull its method-signature map.  Idempotent — a no-op
            // when this protocol was already loaded earlier.
            self.register_stdlib_protocol_for_name(&impl_desc.protocol, metadata);

            // Idempotent guard: skip if THIS impl (target_type,
            // protocol) was already registered.  The protocol-checker
            // uses (type-key, protocol-key) as its impl-index key —
            // duplicate registrations would still overwrite cleanly,
            // but skipping spares the repeated allocation.
            let for_type = Type::Named {
                path: Self::text_to_path(&impl_desc.target_type),
                args: List::new(),
            };
            {
                let pc = self.protocol_checker.read();
                if pc.get_implementations(&for_type).iter().any(|i| {
                    i.protocol
                        .as_ident()
                        .map(|id| id.as_str() == impl_desc.protocol.as_str())
                        .unwrap_or(false)
                }) {
                    proto_deps.push(impl_desc.protocol.clone());
                    continue;
                }
            }

            let associated_types: verum_common::Map<Text, Type> = impl_desc
                .associated_types
                .iter()
                .map(|(name, type_name)| {
                    let ty = Type::Named {
                        path: Self::text_to_path(type_name),
                        args: List::new(),
                    };
                    (name.clone(), ty)
                })
                .collect();

            let methods: verum_common::Map<Text, Type> = {
                let pc = self.protocol_checker.read();
                if let Maybe::Some(protocol) = pc.get_protocol(&impl_desc.protocol) {
                    protocol
                        .methods
                        .iter()
                        .map(|(name, method)| (name.clone(), method.ty.clone()))
                        .collect()
                } else {
                    verum_common::Map::new()
                }
            };

            // Task #23 — parse precompile-captured protocol-arg
            // text-renders into `Type`s so that
            // `ProtocolChecker::can_convert_residual` (and every
            // other arg-aware impl probe) sees the actual
            // protocol-argument shape instead of an empty list.
            //
            // The text comes from
            // `precompile::scan_implementation_protocol_args`, which
            // ran `pretty::format_type` over each
            // `ImplKind::Protocol.protocol_args` entry.  Re-parse
            // through the structural reader so the result is a
            // `Type` that round-trips through unification.
            let protocol_args: List<Type> = impl_desc
                .protocol_args
                .iter()
                .map(|s| crate::infer::helpers::parse_descriptor_type_string(s.as_str()))
                .collect();

            let protocol_impl = ProtocolImpl {
                protocol: Self::text_to_path(&impl_desc.protocol),
                protocol_args,
                for_type,
                where_clauses: List::new(),
                methods,
                associated_types,
                associated_consts: verum_common::Map::new(),
                specialization: Maybe::None,
                impl_crate: Maybe::None,
                span: Span::default(),
                type_param_fn_bounds: verum_common::Map::new(),
            };
            let _ = self.protocol_checker.write().register_impl(protocol_impl);
            proto_deps.push(impl_desc.protocol.clone());
        }
        proto_deps
    }

    /// Walk a parsed Type and push every named-type reference
    /// (Type::Named, Type::Generic.name, Type::Reference.inner …)
    /// into the lazy-loader's pending set.  Ensures that methods
    /// whose signatures reference stdlib type aliases (e.g.
    /// `fs_metadata(p: &Path) -> IoResult<Metadata>`) trigger
    /// loading of IoResult, Metadata, and Path so their alias
    /// targets and inductive-constructor registrations are
    /// available when the user code matches against them.
    /// On-demand stdlib type loader for method-dispatch sites.
    ///
    /// The pre-pass (`register_stdlib_types_for_module`) doesn't
    /// recurse into function bodies — `collect_named_types_from_function_body`
    /// is intentionally a no-op for performance.  Body-local
    /// pattern matches that bind variables of stdlib types (e.g.
    /// `match fs_metadata(p) { Ok(m) => m.len() }`) therefore reach
    /// method dispatch with the receiver type's `inherent_methods`
    /// bucket empty.  This helper closes that gap by walking every
    /// type name reachable from `recv_ty` and lazy-loading each
    /// through the same machinery the pre-pass uses.
    ///
    /// Idempotent — short-circuits at every layer:
    ///  * `ensure_stdlib_type_loaded` skips if `ctx.type_defs`
    ///    already has the type.
    ///  * `register_inherent_methods_from_metadata` skips method
    ///    names already populated in the bucket.
    pub fn lazy_load_receiver_methods(&mut self, recv_ty: &Type) {
        if self.core_metadata.is_none() {
            return;
        }
        let mut names: Vec<Text> = Vec::new();
        Self::push_referenced_type_names(recv_ty, &mut names);
        if names.is_empty() {
            return;
        }
        let mut pending: Vec<Text> = names;
        let mut already: std::collections::HashSet<Text> =
            std::collections::HashSet::new();
        while let Some(name) = pending.pop() {
            if !already.insert(name.clone()) {
                continue;
            }
            self.ensure_stdlib_type_loaded(&name, &mut pending);
        }
    }

    /// CoreMetadata-driven re-export resolver for free functions.
    ///
    /// Walks `ast`'s public mount declarations looking for a tree
    /// that re-exports `func_name`.  When found, derives the source
    /// module path from the mount's prefix and constructs the
    /// `module_path.func_name` key for `metadata.functions`.  If
    /// metadata holds that key, parse the descriptor's parameter
    /// + return strings into a `Type::Function` and return.
    ///
    /// Stops at the first match — multiple re-export chains
    /// shouldn't produce duplicate keys (the first-wins discipline
    /// in archive_metadata gives at most one qualified key per
    /// concrete (module_path, simple_name) pair).
    pub fn resolve_metadata_reexport_function(
        &self,
        metadata: &crate::core_metadata::CoreMetadata,
        ast: &verum_ast::Module,
        func_name: &str,
        ast_module_path: &Text,
    ) -> Option<Type> {
        use verum_ast::ItemKind;
        use verum_ast::decl::{MountTreeKind, Visibility as AstVisibility};
        use verum_ast::ty::PathSegment;
        for item in ast.items.iter() {
            let mount = match &item.kind {
                ItemKind::Mount(m) if m.visibility == AstVisibility::Public => m,
                _ => continue,
            };
            // Two surfaces: `pub mount .X.{name}` (Nested) and
            // `pub mount .X.name` (Path).
            let (matched_prefix_segments, matched) = match &mount.tree.kind {
                MountTreeKind::Nested { prefix, trees } => {
                    let mut found = false;
                    for tree in trees.iter() {
                        if let MountTreeKind::Path(item_path) = &tree.kind {
                            if let Some(PathSegment::Name(id)) = item_path.segments.last() {
                                if id.name.as_str() == func_name {
                                    found = true;
                                    break;
                                }
                            }
                        }
                    }
                    if !found {
                        continue;
                    }
                    let segs: Vec<&str> = prefix
                        .segments
                        .iter()
                        .filter_map(|s| match s {
                            PathSegment::Name(id) => Some(id.name.as_str()),
                            _ => None,
                        })
                        .collect();
                    (segs, true)
                }
                MountTreeKind::Path(path) => {
                    if let Some(PathSegment::Name(id)) = path.segments.last() {
                        if id.name.as_str() != func_name {
                            continue;
                        }
                    } else {
                        continue;
                    }
                    let segs: Vec<&str> = path
                        .segments
                        .iter()
                        .take(path.segments.len() - 1)
                        .filter_map(|s| match s {
                            PathSegment::Name(id) => Some(id.name.as_str()),
                            _ => None,
                        })
                        .collect();
                    (segs, true)
                }
                _ => continue,
            };
            if !matched {
                continue;
            }
            // Build `current_module.prefix.func_name`.  We don't
            // have direct access to the source module's path here,
            // but `self.current_module_path` reflects the unit
            // currently being typechecked — the usual case is the
            // caller passes ast = module_info.ast where module_info
            // is the directly-mounted module, so its path lives in
            // the resolved_module_path the caller already knows.
            // Pass that through current_module_path so this helper
            // can compose the qualified key.
            let module_path = matched_prefix_segments.join(".");
            // Compose candidate qualified keys for the
            // `metadata.functions` lookup.  The function's actual
            // recorded module_path is the SOURCE module — but the
            // function may have been re-exported through any number
            // of intermediate modules, so we have to try multiple
            // assumptions about which module owns the descriptor.
            //
            // Precedence:
            //   1. `<ast_module_path>.<func_name>` — the function
            //      was registered against the re-exporting module
            //      (most common for stdlib's `public module X;` +
            //      `public mount .X.{fn}` pattern, where the VBC
            //      module_path strips the file segment).
            //   2. `<ast_module_path>.<prefix>.<func_name>` — the
            //      function lives in the prefix submodule.
            //   3. `<prefix>.<func_name>` — absolute prefix.
            //   4. `core.<prefix>.<func_name>` — root prefix
            //      where the prefix already starts at `core`.
            let cur = ast_module_path.as_str();
            let mut candidates: Vec<String> = Vec::new();
            if !cur.is_empty() {
                candidates.push(format!("{}.{}", cur, func_name));
                if !module_path.is_empty() {
                    candidates.push(format!("{}.{}.{}", cur, module_path, func_name));
                }
            }
            if !module_path.is_empty() {
                candidates.push(format!("core.{}.{}", module_path, func_name));
                candidates.push(format!("{}.{}", module_path, func_name));
            }
            for key in candidates {
                let key_text: Text = key.into();
                if let Some(fd) = metadata.functions.get(&key_text) {
                    let params: List<Type> = fd
                        .params
                        .iter()
                        .map(|p| parse_descriptor_type_string(p.ty.as_str()))
                        .collect();
                    let return_ty = parse_descriptor_type_string(fd.return_type.as_str());
                    return Some(Type::function(params, return_ty));
                }
            }
        }
        None
    }

    pub(super) fn push_referenced_type_names(ty: &Type, out: &mut Vec<Text>) {
        match ty {
            Type::Named { path, args } => {
                if let Some(seg) = path.segments.last() {
                    if let verum_ast::ty::PathSegment::Name(id) = seg {
                        out.push(Text::from(id.name.as_str()));
                    }
                }
                for a in args.iter() {
                    Self::push_referenced_type_names(a, out);
                }
            }
            Type::Generic { name, args } => {
                out.push(name.clone());
                for a in args.iter() {
                    Self::push_referenced_type_names(a, out);
                }
            }
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => {
                Self::push_referenced_type_names(inner, out)
            }
            Type::Tuple(elems) => {
                for e in elems.iter() {
                    Self::push_referenced_type_names(e, out);
                }
            }
            Type::Function { params, return_type, .. } => {
                for p in params.iter() {
                    Self::push_referenced_type_names(p, out);
                }
                Self::push_referenced_type_names(return_type, out);
            }
            _ => {}
        }
    }

    /// Lazy counterpart of the AST-driven `import_impl_blocks` pass.
    ///
    /// Walks `type_desc.methods` (populated by
    /// `archive_metadata::register_module_metadata` from VBC
    /// `FunctionDescriptor.parent_type`) and registers each
    /// `(method_name → method_type_scheme)` pair into the checker's
    /// `inherent_methods` table — same structure the AST path
    /// populates.  Without this step calls like
    /// `Text.with_capacity(n)` would fail typecheck even though
    /// the body is in the precompiled VBC archive.
    ///
    /// Idempotent: skips method names already present in
    /// `inherent_methods[name]` so repeated
    /// `ensure_stdlib_type_loaded` calls don't accumulate
    /// duplicates.
    pub(super) fn register_inherent_methods_from_metadata(
        &self,
        type_name: &Text,
        type_desc: &crate::core_metadata::TypeDescriptor,
        metadata: &crate::core_metadata::CoreMetadata,
    ) -> Vec<Text> {
        let mut referenced: Vec<Text> = Vec::new();
        if type_desc.methods.is_empty() {
            return referenced;
        }
        let mut methods_guard = self.inherent_methods.write();
        let bucket = methods_guard.entry(type_name.clone()).or_default();
        for method_name in type_desc.methods.iter() {
            if bucket.get(method_name).is_some() {
                continue;
            }
            // Prefer qualified `Type.method` lookup so a simple name
            // shared across multiple types (e.g. `with_capacity` on
            // Text + TextBuilder + List) resolves to the descriptor
            // belonging to THIS type.  Fall back to the bare simple
            // name for free functions / single-type methods.
            let qualified: Text = format!("{}.{}", type_name, method_name).into();
            let fn_desc = match metadata.functions.get(&qualified) {
                Some(d) => d,
                None => match metadata.functions.get(method_name) {
                    Some(d) => d,
                    None => continue,
                },
            };
            // Build the function type from the descriptor.
            // `parse_descriptor_type_string` handles primitives,
            // bare names, generic instantiations
            // (`IoResult<Metadata>`), and references (`&Text`)
            // — required because `archive_metadata::type_ref_to_text`
            // serialises VBC TypeRefs as joined strings, and a
            // naive `Type::Named { path: "IoResult<Metadata>" }`
            // would never unify with the call-site
            // `Type::Generic { name: "IoResult", args: [Metadata] }`.
            let to_type = |s: &Text| -> Type { parse_descriptor_type_string(s.as_str()) };
            // Skip the `self` receiver parameter when present.  VBC
            // stores method descriptors with the receiver as the
            // first parameter (named `"self"`) but its `type_ref`
            // is a `TypeId::UNIT` sentinel — `type_ref_to_text`
            // renders this as `"Unit"`, which would yield a method
            // signature `fn(Unit, …) -> R`.  Verum dispatches the
            // receiver separately during method resolution; the
            // inherent_methods bucket should hold ONLY the
            // call-site arg types.  Skipping `self` here aligns
            // with how `import_impl_blocks` (the AST-driven path)
            // populates the bucket.
            let params: List<Type> = fn_desc
                .params
                .iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    if i == 0 && p.name.as_str() == "self" {
                        None
                    } else {
                        Some(to_type(&p.ty))
                    }
                })
                .collect();
            let return_ty = to_type(&fn_desc.return_type);
            // Recursively collect referenced type names so the
            // lazy loader registers any types this method's
            // signature touches (Path, Metadata, IoResult, …).
            for p in params.iter() {
                Self::push_referenced_type_names(p, &mut referenced);
            }
            Self::push_referenced_type_names(&return_ty, &mut referenced);
            let fn_ty = Type::function(params, return_ty);
            // Determine whether the method is static (no `self`
            // receiver).  Static-method dispatch sites
            // (`Text.with_capacity(64)`, `Heap.alloc(layout)` …)
            // read from the `$static$<method>` bucket key;
            // instance-method dispatch reads the bare key.  The
            // AST-driven registration path elsewhere in this file
            // follows the same convention.  Pre-fix every
            // metadata-loaded static method was registered ONLY
            // under the bare key, so every `Type.static_method(...)`
            // call site failed typecheck despite the body being in
            // the precompiled archive.
            let is_static = fn_desc
                .params
                .first()
                .map(|p| p.name.as_str() != "self")
                .unwrap_or(true);
            // Wrap in a polymorphic TypeScheme when the method's
            // signature contains free TypeVars (introduced by
            // `parse_descriptor_type_string`'s
            // `__generic_N`/`__opaque_type_N` → fresh-TypeVar
            // conversion).  These represent the method's generic
            // parameters: e.g. `Shared.new(value: T) -> Shared<T>`
            // gets `fn(_TyVar_a) -> Shared<_TyVar_a>` after
            // structural parsing.
            //
            // Pre-fix the method was registered as `mono(fn_ty)`,
            // which means the SAME TypeVars are reused on every
            // lookup.  First call site `Shared<Int>.new(42)` binds
            // `_TyVar_a := Int` in the unifier's substitution
            // table; second call site `Shared<Bool>.new(true)`
            // looks up the SAME scheme, sees `_TyVar_a` already
            // bound to `Int`, and rejects `true: Bool` with
            // `expected 'Int', found 'Bool'`.
            //
            // Wrapping with `TypeScheme::poly(bound_vars, fn_ty)`
            // forces fresh instantiation of every bound TypeVar
            // on each `instantiate()` call — the canonical
            // Hindley-Milner discipline already used by the AST-
            // driven impl-block path (`register_impl_method` and
            // `compile_pending_default_methods` produce poly
            // schemes; the metadata-driven path was just missing
            // the wrapper).
            //
            // Stdlib-agnostic — the bound-vars list is harvested
            // from the parsed signature itself via
            // `collect_type_vars` (already used by the
            // dependent-types subsystem), no hardcoded type-name
            // list.
            let scheme = {
                use crate::dependent_helpers::collect_type_vars;
                let vars = collect_type_vars(&fn_ty);
                if vars.is_empty() {
                    TypeScheme::mono(fn_ty.clone())
                } else {
                    let var_list: List<crate::ty::TypeVar> =
                        vars.iter().copied().collect();
                    TypeScheme::poly(var_list, fn_ty.clone())
                }
            };
            bucket.insert(method_name.clone(), scheme.clone());
            if is_static {
                let static_key: Text =
                    format!("$static${}", method_name.as_str()).into();
                bucket.entry(static_key).or_insert(scheme);
            }
        }
        referenced
    }

    /// Convert a StageError from the stage checker to a TypeError.
    /// Used for integrating stage validation into type checking.
    /// Stage coherence: runtime code cannot depend on meta-only values, meta code cannot observe runtime state — Stage Coherence Rule
    pub(super) fn stage_error_to_type_error(err: crate::stage_checker::StageError) -> TypeError {
        use crate::stage_checker::StageError;
        match err {
            StageError::StageMismatch {
                current_stage,
                target_stage,
                expected_stage,
                hint,
                ..
            } => TypeError::Other(Text::from(format!(
                "E1001: stage mismatch in quote expression: current stage is {}, target stage is {} (expected {}). {}",
                current_stage, target_stage, expected_stage, hint
            ))),
            StageError::CrossStageCall {
                caller_stage,
                callee_stage,
                callee_name,
                hint,
                ..
            } => TypeError::Other(Text::from(format!(
                "E1002: cross-stage call: stage {} function cannot directly call stage {} function '{}'. {}",
                caller_stage, callee_stage, callee_name, hint
            ))),
            StageError::StageOverflow {
                used_stage,
                max_stage,
                function_name,
                ..
            } => TypeError::Other(Text::from(format!(
                "E1003: stage overflow: meta({}) exceeds maximum allowed stage {} for function '{}'",
                used_stage, max_stage, function_name
            ))),
            StageError::CyclicStage { cycle, start, .. } => TypeError::Other(Text::from(format!(
                "E1004: cyclic stage dependency starting from '{}': {}",
                start,
                cycle
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ))),
            StageError::InvalidStageEscape {
                escape_stage,
                current_stage,
                valid_range,
                ..
            } => TypeError::Other(Text::from(format!(
                "E1005: invalid stage escape: $(stage {}) is invalid in stage {} context (valid: {})",
                escape_stage, current_stage, valid_range
            ))),
        }
    }

    /// Create a TypeChecker with minimal context (no stdlib types at all).
    ///

    /// STDLIB-AGNOSTIC: This is the most minimal constructor, used when:
    /// - Compiling stdlib itself - types are registered as .vr files are parsed
    /// - Testing type system in isolation
    ///

    /// Contains NO types - not even primitives. Caller must register
    /// all types and methods via the type checker APIs.
    ///

    /// For compiling user code, use `new_with_core()` instead.
    pub fn with_minimal_context() -> Self {
        Self {
            ctx: TypeContext::new_minimal(),
            unifier: Unifier::new(),
            refinement: RefinementChecker::new(Default::default()),
            subtyping: Subtyping::new(),
            const_eval: ConstEvaluator::new(),
            protocol_checker: Shared::new(parking_lot::RwLock::new(ProtocolChecker::new())),
            integer_hierarchy: IntegerHierarchy::new(),
            // STDLIB-AGNOSTIC: Minimal context uses minimal kind inferer
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
            resolved_call_targets: std::collections::HashMap::new(),
        }
    }

    /// Load stdlib types from metadata into the type context.
    ///

    /// This registers all types, protocols, and implementations from
    /// the pre-loaded stdlib.vbca metadata.
    ///

    /// Stdlib bootstrap: dependency-ordered compilation of core .vr modules, type metadata extracted from parsed stdlib files
    pub fn load_stdlib_from_metadata(&mut self, metadata: &crate::core_metadata::CoreMetadata) {
        use crate::core_metadata::TypeDescriptorKind;
        use crate::protocol::{Protocol, ProtocolImpl, ProtocolMethod};
        use crate::ty::Type;
        use verum_common::span::Span;

        // Register types from metadata in source declaration order.
        //

        // The variant_type_names registry uses first-registered-wins semantics,
        // so the order types are registered determines which type owns each
        // variant signature when names overlap (e.g., Result.Ok vs CheckedResult.Ok).
        //

        // Iteration walks `type_declaration_order`, which records insertion order
        // through the metadata pipeline:
        //  archive layer order (Core → Text → Collections → …)
        //  → per-module .vr file declaration order
        //

        // This means stdlib's `Maybe`/`Result`/`Ordering` register before any
        // sibling cog's variant aliases naturally — no hardcoded priority list,
        // no compiler-side stdlib type knowledge.
        //

        // Trailing tail: any type present in `metadata.types` but missing from
        // the order list (defensive — should never happen in practice) is
        // appended in alphabetical order so we still register every type.
        let ordered_types = Self::stdlib_iteration_order(metadata);
        for (name, type_desc) in ordered_types {
            // Convert core_metadata::TypeDescriptor to Type
            let ty = self.type_descriptor_to_type(type_desc);
            self.ctx.define_type(name.clone(), ty.clone());
            // Also register in the type environment so type names can be resolved
            // (e.g., `List<Int>` needs "List" in env, not just in type_defs)
            self.ctx
                .env
                .insert(name.clone(), TypeScheme::mono(ty.clone()));

            // Task #26 — for generic variant types, defer to
            // `register_variant_signature_for_lazy` which performs
            // the fresh-TypeVar substitution for payload positions
            // and registers `__type_var_order_<name>` so subsequent
            // `Result<Int, Int>` / `Maybe<Int>` use sites can
            // substitute through the variant payloads correctly.
            // Without this, the inline variant-building code below
            // produced rigid `Type::Named { "T" }` / `Type::Named
            // { "E" }` payloads — every `let r: Result<Int,Int> =
            // Err(7)` then failed typecheck with `expected 'E',
            // found 'Int'` because `ast_to_generic_type::Type::Variant`
            // substitution (types.rs:1573) couldn't find any
            // `Type::Var`s to map.
            if let TypeDescriptorKind::Variant { cases } = &type_desc.kind {
                if !type_desc.generic_params.is_empty() {
                    let mut pending_payload_deps: Vec<verum_common::Text> = Vec::new();
                    crate::infer::helpers::register_variant_signature_for_lazy(
                        self, name, type_desc, cases, &mut pending_payload_deps,
                    );
                    // Eager loader handles dep ordering globally — the
                    // pending list is per-call diagnostic only.
                    drop(pending_payload_deps);
                    // Skip the inline registration that would overwrite
                    // with rigid Named payloads.
                    if !type_desc.generic_params.is_empty() {
                        self.type_generics_count
                            .insert(name.clone(), type_desc.generic_params.len());
                    }
                    continue;
                }
            }

            // CRITICAL FIX: Register variant type signatures for method lookup
            // This enables methods defined on Maybe<T> to be found when the type
            // has been normalized to its variant form (None | Some(T))
            if let TypeDescriptorKind::Variant { cases } = &type_desc.kind {
                // Build the variant type structure to compute its signature
                let mut variant_map: indexmap::IndexMap<verum_common::Text, Type> =
                    indexmap::IndexMap::new();
                for case in cases.iter() {
                    let payload_ty = match &case.payload {
                        verum_common::Maybe::None => Type::Unit,
                        verum_common::Maybe::Some(crate::core_metadata::VariantPayload::Tuple(
                            types,
                        )) => {
                            if types.len() == 1 {
                                // Single field tuple - unwrap for signature
                                Type::Named {
                                    path: Self::text_to_path(&types[0]),
                                    args: verum_common::List::new(),
                                }
                            } else {
                                // Multiple fields - create tuple
                                Type::Tuple(
                                    types
                                        .iter()
                                        .map(|t| Type::Named {
                                            path: Self::text_to_path(t),
                                            args: verum_common::List::new(),
                                        })
                                        .collect(),
                                )
                            }
                        }
                        verum_common::Maybe::Some(
                            crate::core_metadata::VariantPayload::Record(fields),
                        ) => {
                            let field_map: indexmap::IndexMap<verum_common::Text, Type> = fields
                                .iter()
                                .map(|f| {
                                    (
                                        f.name.clone(),
                                        Type::Named {
                                            path: Self::text_to_path(&f.ty),
                                            args: verum_common::List::new(),
                                        },
                                    )
                                })
                                .collect();
                            Type::Record(field_map)
                        }
                    };
                    variant_map.insert(case.name.clone(), payload_ty);
                }

                let variant_type = Type::Variant(variant_map.clone());
                if let Some(sig) = Self::variant_type_signature(&variant_type) {
                    self.register_variant_type_name_first_wins(sig.clone(), name.clone());
                    if let Some(relaxed_sig) = Self::variant_type_signature_relaxed(&variant_type) {
                        if relaxed_sig != sig {
                            self.register_variant_type_name_first_wins(relaxed_sig, name.clone());
                        }
                    }
                }

                // Register variant constructor parent mappings for scope-aware resolution.
                for (vname, _payload_ty) in &variant_map {
                    let parents = self
                        .variant_constructor_parents
                        .entry(vname.clone())
                        .or_default();
                    if !parents.iter().any(|p| p == name) {
                        parents.push(name.clone());
                    }
                }

                // Register unit variant constructors as values in the env.
                // This enables pattern matching and expression usage of stdlib variant
                // constructors (e.g., `Less`, `Equal`, `Greater` from Ordering).
                for (vname, payload_ty) in &variant_map {
                    if *payload_ty == Type::Unit {
                        // Only register short name if not already taken
                        if self.ctx.env.lookup(vname.as_str()).is_none() {
                            self.ctx
                                .env
                                .insert_mono(vname.clone(), variant_type.clone());
                        }
                    }
                    // Always register qualified name (Type.Variant)
                    let qualified_name: verum_common::Text = format!("{}.{}", name, vname).into();
                    self.ctx
                        .env
                        .insert_mono(qualified_name, variant_type.clone());
                }
            }

            // Register generic parameters count for later instantiation
            if !type_desc.generic_params.is_empty() {
                self.type_generics_count
                    .insert(name.clone(), type_desc.generic_params.len());
            }

            // FUNDAMENTAL #3 — mirror the lazy loader's
            // `__newtype_inner_X` hook here so the eager path
            // (`load_stdlib_from_metadata`) also registers newtype
            // inner-types when it's the active loader.  Without this
            // hook, transparent-wrapper newtypes loaded eagerly
            // (which is the default for the embedded stdlib) miss
            // their inner-type binding and every `.0` access path
            // through the typechecker fails.
            if type_desc.is_transparent_wrapper
                && let TypeDescriptorKind::Record { fields } = &type_desc.kind
                && let Some(first_field) = fields.first()
                && !first_field.ty.is_empty()
            {
                let inner_ty = parse_descriptor_type_string(first_field.ty.as_str());
                let inner_key = format!("__newtype_inner_{}", name);
                self.ctx
                    .define_type(verum_common::Text::from(inner_key.as_str()), inner_ty);
            }
        }

        // Register protocols via protocol_checker (public API)
        // Sort for deterministic registration order (HashMap iteration is non-deterministic).
        let mut sorted_protocols: Vec<_> = metadata.protocols.iter().collect();
        sorted_protocols.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        for (name, protocol_desc) in sorted_protocols {
            // Convert core_metadata methods to protocol ProtocolMethods
            let mut methods = verum_common::Map::new();
            for m in protocol_desc.required_methods.iter() {
                // Build function type from method signature
                let params: verum_common::List<Type> = m
                    .params
                    .iter()
                    .map(|p| {
                        // Parse type name - simplified for now
                        Type::Named {
                            path: Self::text_to_path(&p.ty),
                            args: verum_common::List::new(),
                        }
                    })
                    .collect();
                let return_type = Type::Named {
                    path: Self::text_to_path(&m.return_type.clone()),
                    args: verum_common::List::new(),
                };
                let method_type = Type::function(params, return_type);

                let protocol_method = ProtocolMethod::simple(
                    m.name.clone(),
                    method_type,
                    false, // required methods don't have defaults
                );
                methods.insert(m.name.clone(), protocol_method);
            }

            // Add default methods
            for m in protocol_desc.default_methods.iter() {
                let params: verum_common::List<Type> = m
                    .params
                    .iter()
                    .map(|p| Type::Named {
                        path: Self::text_to_path(&p.ty),
                        args: verum_common::List::new(),
                    })
                    .collect();
                let return_type = Type::Named {
                    path: Self::text_to_path(&m.return_type.clone()),
                    args: verum_common::List::new(),
                };
                let method_type = Type::function(params, return_type);

                let protocol_method = ProtocolMethod::simple(
                    m.name.clone(),
                    method_type,
                    true, // default methods have implementations
                );
                methods.insert(m.name.clone(), protocol_method);
            }

            // Create Protocol struct
            let protocol = Protocol {
                name: name.clone(),
                kind: crate::protocol::ProtocolKind::Constraint,
                type_params: protocol_desc
                    .generic_params
                    .iter()
                    .map(|g| crate::protocol::TypeParam {
                        name: g.name.clone(),
                        bounds: g
                            .bounds
                            .iter()
                            .map(|b| crate::protocol::ProtocolBound {
                                protocol: Self::text_to_path(b),
                                args: verum_common::List::new(),
                                is_negative: false,
                            })
                            .collect(),
                        default: g.default.as_ref().map(|d| Type::Named {
                            path: Self::text_to_path(d),
                            args: verum_common::List::new(),
                        }),
                    })
                    .collect(),
                methods,
                associated_types: verum_common::Map::new(), // Simplified
                associated_consts: verum_common::Map::new(),
                super_protocols: protocol_desc
                    .super_protocols
                    .iter()
                    .map(|sp| crate::protocol::ProtocolBound {
                        protocol: Self::text_to_path(sp),
                        args: verum_common::List::new(),
                        is_negative: false,
                    })
                    .collect(),
                specialization_info: verum_common::Maybe::None,
                defining_crate: verum_common::Maybe::None,
                span: Span::default(),
            };

            let _ = self.protocol_checker.write().register_protocol(protocol);
        }

        // Register protocol implementations via protocol_checker
        for impl_desc in metadata.implementations.iter() {
            // Convert associated types from impl descriptor
            let associated_types: verum_common::Map<verum_common::Text, Type> = impl_desc
                .associated_types
                .iter()
                .map(|(name, type_name)| {
                    let ty = Type::Named {
                        path: Self::text_to_path(type_name),
                        args: verum_common::List::new(),
                    };
                    (name.clone(), ty)
                })
                .collect();

            // Look up the protocol definition to get method types
            // The protocol was registered in the previous loop, so we can look it up
            let methods: verum_common::Map<verum_common::Text, Type> = {
                let protocol_checker_guard = self.protocol_checker.read();
                #[cfg(debug_assertions)]
                if impl_desc.protocol.as_str() == "Ord" {
                    // eprintln!("[DEBUG load_stdlib_impl] Looking up protocol 'Ord' for impl on '{}'", impl_desc.target_type);
                    if let verum_common::Maybe::Some(protocol) =
                        protocol_checker_guard.get_protocol(&impl_desc.protocol)
                    {
                        // eprintln!("[DEBUG load_stdlib_impl] Found protocol 'Ord' with {} methods", protocol.methods.len());
                        for (name, method) in protocol.methods.iter() {
                            // eprintln!("[DEBUG load_stdlib_impl] method '{}': {:?}", name, method.ty);
                        }
                    } else {
                        // eprintln!("[DEBUG load_stdlib_impl] Protocol 'Ord' NOT FOUND!");
                    }
                }
                if let verum_common::Maybe::Some(protocol) =
                    protocol_checker_guard.get_protocol(&impl_desc.protocol)
                {
                    protocol
                        .methods
                        .iter()
                        .map(|(name, method)| (name.clone(), method.ty.clone()))
                        .collect()
                } else {
                    verum_common::Map::new()
                }
            };

            let protocol_impl = ProtocolImpl {
                protocol: Self::text_to_path(&impl_desc.protocol),
                protocol_args: verum_common::List::new(),
                for_type: Type::Named {
                    path: Self::text_to_path(&impl_desc.target_type),
                    args: verum_common::List::new(),
                },
                where_clauses: verum_common::List::new(),
                methods,
                associated_types,
                associated_consts: verum_common::Map::new(),
                specialization: verum_common::Maybe::None,
                impl_crate: verum_common::Maybe::None,
                span: Span::default(),
                type_param_fn_bounds: verum_common::Map::new(),
            };
            // Ignore coherence errors during metadata loading
            let _ = self.protocol_checker.write().register_impl(protocol_impl);
        }

        // Register inductive constructors for pattern matching
        self.register_stdlib_constructors_from_metadata(metadata);
        // #97 — register stdlib consts as values; mirrors the lazy
        // path above so bare `let s = SSO_CAPACITY;` resolves through
        // env.lookup in eager mode too.
        self.register_stdlib_consts_from_metadata(metadata);
    }

    /// Helper function to create a Path from a type name string.
    pub(super) fn text_to_path(name: &verum_common::Text) -> verum_ast::ty::Path {
        use verum_ast::ty::{Ident, Path};
        use verum_common::span::Span;
        let ident = Ident::new(name.as_str(), Span::default());
        Path::single(ident)
    }

    /// Walk metadata types in stdlib source declaration order.
    ///

    /// Returns `(name, descriptor)` pairs ordered first by `type_declaration_order`
    /// (which records insertion order: archive layer → .vr declaration order),
    /// then any orphan types not present in that list appended in alphabetical
    /// (BTreeMap) order. The orphan tail is defensive — every type inserted via
    /// `core_loader::extract_module_metadata` or `pipeline::cached → metadata`
    /// is already pushed to `type_declaration_order`.
    ///

    /// First-registered-wins iteration is the architectural alternative to
    /// hardcoded priority lists like `["Result", "Maybe", "Ordering", "Bool"]`.
    /// Compiler stays stdlib-agnostic; correctness comes from source order.
    pub(super) fn stdlib_iteration_order(
        metadata: &crate::core_metadata::CoreMetadata,
    ) -> Vec<(&Text, &crate::core_metadata::TypeDescriptor)> {
        let mut seen: std::collections::HashSet<&Text> =
            std::collections::HashSet::with_capacity(metadata.types.len());
        let mut out: Vec<(&Text, &crate::core_metadata::TypeDescriptor)> =
            Vec::with_capacity(metadata.types.len());

        for name in metadata.type_declaration_order.iter() {
            if let Some(desc) = metadata.types.get(name) {
                if seen.insert(name) {
                    out.push((name, desc));
                }
            }
        }

        for (name, desc) in metadata.types.iter() {
            if !seen.contains(name) {
                seen.insert(name);
                out.push((name, desc));
            }
        }

        out
    }

    /// Convert a core_metadata::TypeDescriptor to a Type.
    fn type_descriptor_to_type(&self, desc: &crate::core_metadata::TypeDescriptor) -> Type {
        use crate::core_metadata::{TypeDescriptorKind, VariantPayload};
        use crate::ty::Type;
        use verum_common::Text;

        // Task #39 — Variant descriptors must produce a real
        // `Type::Variant(case_payload_map)`, not a bare `Type::Named`.
        // The record-literal typechecker at `infer_expr_record`
        // (line ~7773) probes `ctx.lookup_type` and expects
        // `Type::Variant(variants)` form to validate
        // `RetryBackoff.Jittered { base_ms: …, max_ms: … }`
        // record-payload constructor.  Pre-fix every stdlib variant
        // type lazy-loaded via metadata appeared as `Type::Named`,
        // so the variant-case detection skipped to the single-
        // segment-variant fallback which mis-types the constructor's
        // result as the qualified case path (`RetryBackoff.Jittered`)
        // instead of the parent variant type (`RetryBackoff`).
        // Downstream method dispatch then fails with
        // `no method named delay_for_attempt for type
        // RetryBackoff.Jittered`.
        // Task #41 — parse payload types via the structural parser so
        // generic instantiations like "List<Byte>" / "Maybe<Text>" /
        // "Result<T, E>" become proper `Type::Named { path, args }`,
        // not a single Ident with the whole generic name baked in.
        let parse = crate::infer::helpers::parse_descriptor_type_string;
        let build_variant_payloads = |cases: &verum_common::List<
            crate::core_metadata::VariantCase,
        >|
         -> indexmap::IndexMap<Text, Type> {
            let mut map: indexmap::IndexMap<Text, Type> = indexmap::IndexMap::new();
            for case in cases.iter() {
                let payload_ty = match &case.payload {
                    verum_common::Maybe::None => Type::Unit,
                    verum_common::Maybe::Some(VariantPayload::Tuple(types)) => {
                        if types.len() == 1 {
                            parse(types[0].as_str())
                        } else {
                            // Multi-arg tuple payload — wrap in Tuple.
                            let inner: verum_common::List<Type> =
                                types.iter().map(|t| parse(t.as_str())).collect();
                            Type::Tuple(inner)
                        }
                    }
                    verum_common::Maybe::Some(VariantPayload::Record(fields)) => {
                        let mut field_map: indexmap::IndexMap<Text, Type> =
                            indexmap::IndexMap::new();
                        for f in fields.iter() {
                            field_map
                                .insert(Text::from(f.name.as_str()), parse(f.ty.as_str()));
                        }
                        Type::Record(field_map)
                    }
                };
                map.insert(case.name.clone(), payload_ty);
            }
            map
        };

        if desc.generic_params.is_empty() {
            // Concrete type
            match &desc.kind {
                TypeDescriptorKind::Record { .. } => Type::Named {
                    path: Self::text_to_path(&desc.name),
                    args: verum_common::List::new(),
                },
                TypeDescriptorKind::Variant { cases } => {
                    Type::Variant(build_variant_payloads(cases))
                }
                TypeDescriptorKind::Protocol { .. } => Type::Named {
                    path: Self::text_to_path(&desc.name),
                    args: verum_common::List::new(),
                },
                TypeDescriptorKind::Alias { target } => Type::Named {
                    path: Self::text_to_path(target),
                    args: verum_common::List::new(),
                },
                TypeDescriptorKind::Opaque => Type::Named {
                    path: Self::text_to_path(&desc.name),
                    args: verum_common::List::new(),
                },
            }
        } else {
            // Generic type — for variants we still want the
            // Type::Variant form so record-payload literals
            // typecheck; the generic params surface via fresh
            // TypeVars inside each case payload.  Tuple/Record
            // payload types that reference the generic params get
            // their fresh-var substitution in the
            // `try_resolve_variant_constructor` path; this
            // descriptor returns the structural shape only.
            match &desc.kind {
                TypeDescriptorKind::Variant { cases } => {
                    Type::Variant(build_variant_payloads(cases))
                }
                _ => Type::Generic {
                    name: desc.name.clone(),
                    args: desc
                        .generic_params
                        .iter()
                        .map(|_| Type::Var(crate::ty::TypeVar::fresh()))
                        .collect(),
                },
            }
        }
    }

    // Environment management methods (register_stdlib*, pre_register_module*, defer_constraint*, …)
    // → see infer/env.rs in this module
    // Type resolution and normalization methods (ast_to_type*, normalize_type*, check_cast, …)
    // → see infer/types.rs in this module
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}
