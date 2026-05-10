//! ATS-V architectural type system phase.
//!
//! This is the compiler-pipeline integration of the kernel-side
//! `verum_kernel::arch_phase` module.  The phase runs after
//! type-checking and walks every `@arch_module(...)` declaration
//! in the module — both the module-level attribute and per-item
//! attributes (cog / module / function level).
//!
//! For each declaration:
//!
//!   1. Extract the named-arg expressions from the `@arch_module(...)`
//!      attribute.
//!   2. Hand them to `verum_kernel::arch_phase::run_arch_phase_one`,
//!      which parses them into a `Shape` and runs the full canonical
//!      anti-pattern checker (32 patterns: AP-001..032).
//!   3. Each `AntiPatternViolation` becomes a compiler diagnostic
//!      carrying the stable RFC code (ATS-V-AP-NNN) + the human-
//!      readable message + the `auto_fix_suggestion` (when present)
//!      as a help label.
//!
//! Backward compatibility: modules without an `@arch_module(...)`
//! declaration are silently skipped.  The default Shape has empty
//! capability lists, ZFC foundation, multi-tier execution, etc.,
//! so it would vacuously pass every anti-pattern check anyway —
//! emitting diagnostics for unannotated code would just generate
//! noise during the gradual ATS-V rollout.

use anyhow::Result;
use tracing::debug;

use verum_ast::Module;
use verum_common::{List, Maybe};
use verum_diagnostics::{DiagnosticBuilder, Severity};
use verum_kernel::arch::Foundation;
use verum_kernel::arch_anti_pattern::{AntiPatternViolation, Severity as KernelSeverity};
use verum_kernel::arch_phase::{run_arch_phase_one_with, ModuleArchResult, PhaseInputs};

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    /// ATS-V phase — runs the architectural-type checker over every
    /// `@arch_module(...)` declaration in the module.  Emits one
    /// diagnostic per violation; never fails the build directly
    /// (the diagnostic stream's `abort_if_errors` handles aggregation).
    pub(super) fn phase_ats_v(&self, module: &Module) -> Result<()> {
        let mut total_violations = 0usize;

        // Aggregate every `@framework(corpus, "...")` annotation
        // across the module — both at the module level and on
        // each item — so AP-026 FoundationContentMismatch fires
        // when ANY body construct cites a foreign foundation, not
        // just citations attached to the module declaration itself.
        let module_wide_foreign_constructs =
            collect_module_wide_foreign_foundations(module);

        // Body-level capability inference (Q5).  Walks every
        // function body in the module, matches each call site
        // against the canonical ontology in
        // `verum_kernel::arch_capability_inference`, and aggregates
        // the inferred Capability set.  Activates AP-001
        // CapabilityEscalation in production builds.
        let inferred_used_capabilities = infer_used_capabilities(module);

        // 1. Module-level @arch_module(...) — the primary surface.
        //   Use the registry-aware entry so cross-cog peer resolution
        //   (composed_foundations / cited_lifecycles / callee_tiers)
        //   + body-level capability inference activate AP-001 /
        //   AP-004 / AP-005 / AP-009 in production.
        if let Some(result) = self.run_arch_phase_for_attrs_registry_aware(
            &module.attributes,
            "<module>",
            &module_wide_foreign_constructs,
            &inferred_used_capabilities,
        ) {
            total_violations += result.violations.len();
            self.emit_arch_phase_result(&result, module);
        }

        // 2. Per-item @arch_module(...) — cog / module / function
        //    level annotations (per spec §17.4 the attribute can be
        //    attached to any module-shaped item).
        for item in &module.items {
            let item_name = item_display_name(item);
            // Outer item.attributes — checked first because they
            // generally carry the user-facing meta (e.g. @derive,
            // @arch_module).  Per-decl inner attributes (decl_attrs)
            // we skip here because @arch_module is conventionally
            // an outer item attribute.
            //
            // For items, foreign-foundation constructs and inferred
            // capabilities are scoped to the item's own body — the
            // module-wide aggregates apply only at the module level.
            let item_foreign_constructs =
                extract_foreign_foundation_constructs(&item.attributes);
            let item_inferred_caps = infer_used_capabilities_in_item(item);
            if let Some(result) = self.run_arch_phase_for_attrs_registry_aware(
                &item.attributes,
                &item_name,
                &item_foreign_constructs,
                &item_inferred_caps,
            ) {
                total_violations += result.violations.len();
                self.emit_arch_phase_result(&result, module);
            }
        }

        if total_violations > 0 {
            debug!(
                "ATS-V phase: {} anti-pattern violations across module",
                total_violations
            );
        }
        Ok(())
    }

    /// Registry-aware variant of `run_arch_phase_for_attrs`.  Reads
    /// the per-attribute Shape, registers it into the session-level
    /// arch-shape registry, then performs cross-cog peer resolution
    /// for `composes_with` to populate
    /// `PhaseInputs.composed_foundations` / `cited_lifecycles` /
    /// `callee_tiers` from peer Shapes already in the registry.
    ///
    /// Order-dependence: a peer processed AFTER this module gets a
    /// `None` lookup and its check skips.  No false-positive — the
    /// registry is "best-effort known-at-this-point".
    fn run_arch_phase_for_attrs_registry_aware(
        &self,
        attrs: &List<verum_ast::attr::Attribute>,
        module_name: &str,
        foreign_foundation_constructs: &[(String, verum_kernel::arch::Foundation)],
        inferred_used_capabilities: &[verum_kernel::arch::Capability],
    ) -> Option<ModuleArchResult> {
        // First pass: locate the @arch_module attribute and parse it
        // to extract the Shape.  We need the Shape's composes_with /
        // foundation / lifecycle / at_tier BEFORE running the check
        // so we can both register it and resolve peers.
        let mut arch_module_args: Option<&[verum_ast::expr::Expr]> = None;
        for attr in attrs.iter() {
            if attr.name.as_str() == "arch_module" {
                arch_module_args = Some(match &attr.args {
                    Maybe::Some(args) => args.as_slice(),
                    Maybe::None => &[],
                });
            }
        }
        let args_slice = arch_module_args?;

        // Parse Shape upfront so we can resolve peers from
        // composes_with.  parse_arch_module is the same path the
        // kernel-side run_arch_phase_one_with would take.
        let parsed_shape = verum_kernel::arch_parse::parse_arch_module(args_slice).ok();

        // Register THIS module's Shape into the session.
        if let Some(shape) = parsed_shape.as_ref() {
            self.session
                .register_arch_shape(module_name.to_string(), shape.clone());
        }

        // Resolve cross-cog peer data from registry.  Best-effort
        // under single-pass architecture.
        let (composed_foundations, cited_lifecycles, callee_tiers) =
            if let Some(shape) = parsed_shape.as_ref() {
                (
                    self.session.resolve_composed_foundations(&shape.composes_with),
                    self.session.resolve_cited_lifecycles(&shape.composes_with),
                    self.session.resolve_callee_tiers(&shape.composes_with),
                )
            } else {
                (Vec::new(), Vec::new(), Vec::new())
            };

        // Transitive multi-hop checks (AP-019 + AP-024) using the
        // session arch-shape registry.  Best-effort under single-pass:
        // peers not yet processed get None lookup; check skips.
        let (transitive_lifecycle_regressions, foundation_downgrades) =
            if let Some(shape) = parsed_shape.as_ref() {
                (
                    self.session
                        .resolve_transitive_lifecycle_regressions(
                            module_name,
                            shape.lifecycle.rank(),
                        ),
                    self.session
                        .resolve_foundation_downgrades(module_name, &shape.foundation),
                )
            } else {
                (Vec::new(), Vec::new())
            };

        // Body-level capability inference (Q5).  The walker is
        // invoked at the CompilationPipeline::phase_ats_v level
        // where the full `Module` AST is available.  Per-attribute
        // helper here gets an empty vec; the module-level call site
        // overrides via the explicit `inferred_used_capabilities`
        // parameter we will pass through `phase_ats_v`.
        let inputs = PhaseInputs {
            capability_ontology_registry: None,
            yoneda_verdicts_claimed: Vec::new(),
            foreign_foundation_constructs: foreign_foundation_constructs.to_vec(),
            composed_foundations,
            cited_lifecycles,
            callee_tiers,
            inferred_used_capabilities: inferred_used_capabilities.to_vec(),
            transitive_lifecycle_regressions,
            foundation_downgrades,
        };
        Some(verum_kernel::arch_phase::run_arch_phase_one_with(
            module_name.to_string(),
            args_slice,
            &inputs,
        ))
    }

    /// Lower one `ModuleArchResult` (one parse + check pass) into
    /// the compiler diagnostic stream.
    fn emit_arch_phase_result(&self, result: &ModuleArchResult, module: &Module) {
        // Parse errors first — these block any anti-pattern reasoning
        // since the Shape is unparseable.
        for parse_err in &result.parse_errors {
            let msg = format!(
                "[ATS-V] @arch_module parse error in `{}`: {:?}",
                result.module_name, parse_err,
            );
            let mut builder = DiagnosticBuilder::new(Severity::Error).message(msg);
            let span = self.session.convert_span(module.span);
            builder = builder.span(span);
            self.session.emit_diagnostic(builder.build());
        }

        // Per-violation diagnostics carrying the stable RFC code.
        for v in &result.violations {
            let diagnostic = build_violation_diagnostic(v, &result.module_name, module, self);
            self.session.emit_diagnostic(diagnostic);
        }
    }
}

/// Build a structured diagnostic from an `AntiPatternViolation`.
/// Under the dual-audience contract: the diagnostic carries the
/// stable code (ATS-V-AP-NNN) so both human reviewers and agents
/// can pattern-match against the same payload.
fn build_violation_diagnostic(
    v: &AntiPatternViolation,
    module_name: &str,
    module: &Module,
    pipeline: &CompilationPipeline<'_>,
) -> verum_diagnostics::Diagnostic {
    let severity = match v.severity {
        KernelSeverity::Error => Severity::Error,
        KernelSeverity::Warning => Severity::Warning,
        KernelSeverity::Hint => Severity::Help,
    };
    let main_msg = format!(
        "[ATS-V {}] {} (in `{}`): {}",
        v.code.code(),
        v.code.name(),
        module_name,
        v.summary,
    );

    let mut builder = DiagnosticBuilder::new(severity).message(main_msg);
    let span = pipeline.session.convert_span(module.span);
    builder = builder.span(span);

    // Append the human-readable explanation as a help label so
    // downstream UIs surface it without parsing the main message.
    if !v.human_message.is_empty() {
        builder = builder.add_note(v.human_message.clone());
    }

    // The auto-fix suggestion (when present) — agents pattern-
    // match on this for autonomous remediation under the
    // dual-audience contract.
    if let Some(fix) = &v.auto_fix_suggestion {
        builder = builder.add_note(format!("Suggested fix: {}", fix));
    }

    // Stable docs URL — carried verbatim into agent surfaces.
    builder = builder.add_note(format!("docs: {}", v.code.docs_url()));

    builder.build()
}


// =============================================================================
// Q5 — Body-level capability inference (AP-001 production wiring)
// =============================================================================

/// Walk every function body in the module, collecting Capability
/// values implied by each call-site whose path is in the canonical
/// ontology.  Returns deduplicated capabilities in stable order.
///
/// Resolution scope (v1):
///   * Recognises `Call { func: Path(...), .. }` with a fully-
///     qualified path matching an `arch_capability_inference`
///     ontology entry.
///   * Skips method calls (`obj.method(...)`) — symbol-table
///     resolution required for type-aware lookup, scheduled for v2.
///   * Skips closures, indirect calls (`fn_ptr(args)`) — same
///     reason.
///
/// Coverage tradeoff: explicit-path calls produce zero false-
/// positives (ontology match is exact); ambiguous resolution
/// silently falls through, producing an empty list (silent path
/// for AP-001 — no violation reported).
pub(crate) fn infer_used_capabilities(
    module: &verum_ast::Module,
) -> Vec<verum_kernel::arch::Capability> {
    use std::collections::HashSet;
    let mut found: HashSet<verum_kernel::arch::Capability> = HashSet::new();
    for item in &module.items {
        walk_item_body_for_caps(item, &mut found);
    }
    found.into_iter().collect()
}

/// Single-item variant — walks only the given item's body.  Used
/// when `phase_ats_v` runs the per-item registry-aware check.
pub(crate) fn infer_used_capabilities_in_item(
    item: &verum_ast::Item,
) -> Vec<verum_kernel::arch::Capability> {
    use std::collections::HashSet;
    let mut found: HashSet<verum_kernel::arch::Capability> = HashSet::new();
    walk_item_body_for_caps(item, &mut found);
    found.into_iter().collect()
}

fn walk_item_body_for_caps(
    item: &verum_ast::Item,
    sink: &mut std::collections::HashSet<verum_kernel::arch::Capability>,
) {
    use verum_ast::decl::{FunctionBody, ItemKind};
    if let ItemKind::Function(fn_decl) = &item.kind {
        if let Maybe::Some(body) = &fn_decl.body {
            match body {
                FunctionBody::Block(block) => walk_block_for_caps(block, sink),
                FunctionBody::Expr(expr) => walk_expr_for_caps(expr, sink),
            }
        }
    }
}

fn walk_block_for_caps(
    block: &verum_ast::Block,
    sink: &mut std::collections::HashSet<verum_kernel::arch::Capability>,
) {
    for stmt in block.stmts.iter() {
        walk_stmt_for_caps(stmt, sink);
    }
    if let Maybe::Some(tail) = &block.expr {
        walk_expr_for_caps(tail, sink);
    }
}

fn walk_stmt_for_caps(
    stmt: &verum_ast::stmt::Stmt,
    sink: &mut std::collections::HashSet<verum_kernel::arch::Capability>,
) {
    use verum_ast::stmt::StmtKind;
    match &stmt.kind {
        StmtKind::Expr { expr, .. } => walk_expr_for_caps(expr, sink),
        StmtKind::Let { value, .. } => {
            if let Maybe::Some(init) = value {
                walk_expr_for_caps(init, sink);
            }
        }
        StmtKind::LetElse { value, .. } => walk_expr_for_caps(value, sink),
        StmtKind::Defer(expr) | StmtKind::Errdefer(expr) => walk_expr_for_caps(expr, sink),
        StmtKind::Provide { value, .. } | StmtKind::ProvideScope { value, .. } => {
            walk_expr_for_caps(value, sink);
        }
        // Item / Empty / etc. — no body-level call sites.
        _ => {}
    }
}

fn walk_expr_for_caps(
    expr: &verum_ast::expr::Expr,
    sink: &mut std::collections::HashSet<verum_kernel::arch::Capability>,
) {
    use verum_ast::expr::ExprKind;
    match &expr.kind {
        ExprKind::Call { func, args, .. } => {
            // Try to resolve the callee path against the canonical
            // ontology.  Only fully-qualified Path expressions match;
            // closures, dynamic dispatches, and method receivers
            // produce None (silent skip).
            if let Some(path) = expr_to_dotted_path(func) {
                if let Some(cap) =
                    verum_kernel::arch_capability_inference::lookup_capability(&path)
                {
                    sink.insert(cap);
                }
            }
            walk_expr_for_caps(func, sink);
            for a in args.iter() {
                walk_expr_for_caps(a, sink);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            // Method-call resolution requires the symbol table for
            // type-aware lookup.  v1 walks the receiver + args for
            // nested calls but does not attribute the method itself.
            walk_expr_for_caps(receiver, sink);
            for a in args.iter() {
                walk_expr_for_caps(a, sink);
            }
        }
        ExprKind::Block(block) => walk_block_for_caps(block, sink),
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            // IfCondition is structurally complex (let-bindings,
            // multiple guard clauses).  v1 walks the branches but
            // skips the condition payload — typical condition
            // expressions don't introduce capability-relevant
            // side effects.
            walk_block_for_caps(then_branch, sink);
            if let Maybe::Some(else_b) = else_branch {
                walk_expr_for_caps(else_b, sink);
            }
        }
        ExprKind::Match { expr: scrut, arms } => {
            walk_expr_for_caps(scrut, sink);
            for arm in arms.iter() {
                walk_expr_for_caps(&arm.body, sink);
            }
        }
        ExprKind::While {
            condition, body, ..
        } => {
            walk_expr_for_caps(condition, sink);
            walk_block_for_caps(body, sink);
        }
        ExprKind::For { iter, body, .. } => {
            walk_expr_for_caps(iter, sink);
            walk_block_for_caps(body, sink);
        }
        ExprKind::Loop { body, .. } => walk_block_for_caps(body, sink),
        ExprKind::Binary { left, right, .. } => {
            walk_expr_for_caps(left, sink);
            walk_expr_for_caps(right, sink);
        }
        ExprKind::Unary { expr: inner, .. } => walk_expr_for_caps(inner, sink),
        ExprKind::Field { expr: inner, .. }
        | ExprKind::OptionalChain { expr: inner, .. }
        | ExprKind::TupleIndex { expr: inner, .. } => walk_expr_for_caps(inner, sink),
        ExprKind::Index { expr: e, index } => {
            walk_expr_for_caps(e, sink);
            walk_expr_for_caps(index, sink);
        }
        ExprKind::Tuple(items) => {
            for e in items.iter() {
                walk_expr_for_caps(e, sink);
            }
        }
        // Leaf / non-recursive arms: Path, Literal, identifiers,
        // closures, etc.  v1 does not enter closure bodies — the
        // capability used at the closure invocation site is
        // captured when the call to the closure is itself walked.
        _ => {}
    }
}

/// Extract a dotted path from an expression of `ExprKind::Path(...)`.
/// Returns `Some("core.io.fs.read_file")` for paths and `None` for
/// anything else.  Used by the capability walker to resolve
/// `Call { func: Path(...), .. }` against the ontology.
fn expr_to_dotted_path(expr: &verum_ast::expr::Expr) -> Option<String> {
    use verum_ast::expr::ExprKind;
    use verum_ast::ty::PathSegment;
    match &expr.kind {
        ExprKind::Path(p) => {
            let segs: Vec<&str> = p
                .segments
                .iter()
                .filter_map(|s| match s {
                    PathSegment::Name(ident) => Some(ident.name.as_str()),
                    _ => None,
                })
                .collect();
            if segs.is_empty() {
                None
            } else {
                Some(segs.join("."))
            }
        }
        _ => None,
    }
}

/// Walk the entire module — both module-level attributes AND
/// every item's attributes — collecting every `@framework(corpus,
/// ...)` annotation.  Used by `phase_ats_v` to feed AP-026
/// FoundationContentMismatch with the complete set of foreign-
/// foundation citations across the module body, not just those
/// attached to the module declaration.
///
/// Q2 closure — without this aggregation, AP-026 only fires on
/// citations directly on the module-level `@arch_module(...)` site.
/// A function deep in the body that cites `@framework(hott, ...)`
/// would be invisible to the cog-level check.
fn collect_module_wide_foreign_foundations(
    module: &Module,
) -> Vec<(String, verum_kernel::arch::Foundation)> {
    let mut out = extract_foreign_foundation_constructs(&module.attributes);
    for item in &module.items {
        out.extend(extract_foreign_foundation_constructs(&item.attributes));
    }
    out
}

/// Walk an attribute list and surface every `@framework(corpus, ...)`
/// annotation as a `(construct_label, foundation_tag)` pair for
/// AP-026 FoundationContentMismatch.
///
/// The translation table maps the `corpus` first-arg of
/// `@framework(corpus, ...)` to the matching `Foundation` enum
/// variant.  Unrecognised corpus names are silently skipped (they
/// will be picked up by AP-023 FoundationForgery via the citation
/// table independently).
fn extract_foreign_foundation_constructs(
    attrs: &List<verum_ast::attr::Attribute>,
) -> Vec<(String, Foundation)> {
    let mut out: Vec<(String, Foundation)> = Vec::new();
    for attr in attrs.iter() {
        if attr.name.as_str() != "framework" {
            continue;
        }
        let args = match &attr.args {
            Maybe::Some(a) => a,
            Maybe::None => continue,
        };
        // First arg is the corpus identifier (e.g. `hott`, `cic`,
        // `mltt`); subsequent args are the citation string.
        let corpus_arg = match args.iter().next() {
            Some(a) => a,
            None => continue,
        };
        let corpus_name = match expr_to_path_str(corpus_arg) {
            Some(s) => s,
            None => continue,
        };
        let foundation = match corpus_name.as_str() {
            "hott" => Foundation::Hott,
            "cubical" => Foundation::Cubical,
            "cic" => Foundation::Cic,
            "mltt" => Foundation::Mltt,
            "eff" => Foundation::Eff,
            "zfc_two_inacc" | "zfc" => Foundation::ZfcTwoInacc,
            // Other corpus names (lurie_htt, schreiber_dcct, etc.)
            // are tracked by AP-023 FoundationForgery directly.
            _ => continue,
        };
        // Citation-string second arg (if present) gives the
        // construct label; fallback uses the corpus name itself.
        let label = args
            .iter()
            .nth(1)
            .and_then(expr_to_string_lit)
            .unwrap_or_else(|| corpus_name.clone());
        out.push((label, foundation));
    }
    out
}

/// Best-effort path-string extraction from an attribute argument.
/// Recognises `Path(["foo"])` and `Path(["foo", "bar"])` shapes.
fn expr_to_path_str(expr: &verum_ast::expr::Expr) -> Option<String> {
    use verum_ast::expr::ExprKind;
    use verum_ast::ty::PathSegment;
    match &expr.kind {
        ExprKind::Path(p) => {
            let segs: Vec<&str> = p
                .segments
                .iter()
                .filter_map(|s| match s {
                    PathSegment::Name(ident) => Some(ident.name.as_str()),
                    _ => None,
                })
                .collect();
            if segs.is_empty() {
                None
            } else {
                Some(segs.last().copied().unwrap_or("").to_string())
            }
        }
        _ => None,
    }
}

/// Best-effort string-literal extraction from an attribute argument.
fn expr_to_string_lit(expr: &verum_ast::expr::Expr) -> Option<String> {
    use verum_ast::expr::ExprKind;
    use verum_ast::literal::{LiteralKind, StringLit};
    match &expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Text(StringLit::Regular(s) | StringLit::MultiLine(s)) => {
                Some(s.as_str().to_string())
            }
            _ => None,
        },
        _ => None,
    }
}


/// Best-effort display name for a top-level item — used in
/// diagnostics so the user knows which declaration carried the
/// `@arch_module(...)` attribute.
fn item_display_name(item: &verum_ast::Item) -> String {
    use verum_ast::decl::ItemKind;
    match &item.kind {
        ItemKind::Function(d) => d.name.name.as_str().to_string(),
        ItemKind::Type(d) => d.name.name.as_str().to_string(),
        ItemKind::Theorem(d) | ItemKind::Lemma(d) | ItemKind::Corollary(d) => {
            d.name.name.as_str().to_string()
        }
        ItemKind::Axiom(d) => d.name.name.as_str().to_string(),
        ItemKind::Const(d) => d.name.name.as_str().to_string(),
        ItemKind::Static(d) => d.name.name.as_str().to_string(),
        ItemKind::Module(d) => d.name.name.as_str().to_string(),
        _ => "<item>".to_string(),
    }
}
