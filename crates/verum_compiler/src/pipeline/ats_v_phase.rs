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
use verum_kernel::arch_phase::{ModuleArchResult, PhaseInputs};

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    /// ATS-V phase — runs the architectural-type checker over every
    /// `@arch_module(...)` declaration in the module.  Emits one
    /// diagnostic per violation; never fails the build directly
    /// (the diagnostic stream's `abort_if_errors` handles aggregation).
    pub(super) fn phase_ats_v(&self, module: &Module) -> Result<()> {
        let mut total_violations = 0usize;

        // ATS-V REACH ORACLE.  `VERUM_TRACE_ATSV=1` reports what this
        // phase can SEE — the count of module-level and per-item
        // attributes, by name.  Named for the question it answers:
        // "did the declaration reach the checker at all", which is
        // distinct from "did the checker accept it". The phase was
        // silent for both reasons at different times (T0834).
        if std::env::var_os("VERUM_TRACE_ATSV").is_some() {
            let mod_attrs: Vec<&str> =
                module.attributes.iter().map(|a| a.name.as_str()).collect();
            eprintln!(
                "[ats-v] module attrs: {:?}; items: {}",
                mod_attrs,
                module.items.len()
            );
            for item in &module.items {
                let names: Vec<&str> =
                    item.attributes.iter().map(|a| a.name.as_str()).collect();
                if !names.is_empty() {
                    eprintln!("[ats-v]   item {} attrs: {:?}", item_display_name(item), names);
                }
            }
        }

        // Aggregate every `@framework(corpus, "...")` annotation
        // across the module — both at the module level and on
        // each item — so AP-026 FoundationContentMismatch fires
        // when ANY body construct cites a foreign foundation, not
        // just citations attached to the module declaration itself.
        let module_wide_foreign_constructs =
            collect_module_wide_foreign_foundations(module);

        // Body-level capability inference — ROW-BASED (T0848).
        // Facts are extracted per function (ontology atoms, local
        // call-graph edges, mixed fn-typed params) and solved by the
        // kernel's SCC fixpoint: a helper's `Network` now reaches its
        // caller's surface TRANSITIVELY, where the previous flat walk
        // saw only direct primitive calls. The module surface feeds
        // the same AP-001 plumbing as before.
        // Local protocol max-Shapes register FIRST (a protocol and
        // its consumer may share the module), then extraction runs
        // with SESSION-backed resolvers: mounted callees resolve to
        // earlier modules' solved summaries — the surface is
        // transitive ACROSS module boundaries, in compilation order.
        for (proto, row) in collect_protocol_max_shapes(module) {
            self.session.register_arch_protocol_max_shape(proto, row);
        }
        let session = &self.session;
        let resolvers = ExtractionResolvers {
            imports: &|q: &str| session.lookup_arch_fn_summary(q),
            protocol_max_shape: &|n: &str| session.lookup_arch_protocol_max_shape(n),
        };
        let module_summaries = infer_module_summaries_with(module, &resolvers);
        // Install THIS module's summaries for everyone after us.
        let module_prefix = module
            .items
            .iter()
            .find_map(|i| match &i.kind {
                verum_ast::decl::ItemKind::Module(m) => Some(m.name.name.to_string()),
                _ => None,
            });
        if let Some(prefix) = &module_prefix {
            for (fn_name, row) in &module_summaries.summaries {
                self.session
                    .register_arch_fn_summary(format!("{prefix}.{fn_name}"), row.clone());
            }
        }
        let inferred_used_capabilities: Vec<verum_kernel::arch::Capability> = module_summaries
            .module_surface()
            .facts()
            .map(|f| f.atom.clone())
            .collect();

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
    // T0866: an architectural verdict is a JUDGMENT, not a type
    // error. Compilation answers "is this program meaningful?";
    // the architecture audit answers "does this program hold the
    // shape it claims?". Conflating them gave the worst of both: a
    // module whose only fault was an unclosed CVE obligation became
    // UNCOMPILABLE, its registration failed, and every dependent
    // module lost the failed module's names — 49 architectural
    // verdicts in the registry corpus cascaded into 110 spurious
    // "not a function" errors that hid the real ones.
    //
    // The strictness does not go away, it moves to the instance that
    // owns it: `verum arch` / `verum audit` judge the shape and carry
    // the exit code CI gates on. Here the verdict is a warning that
    // names its stable code, so the reader still sees it in place.
    // (T0834's lesson — 2311 unearned `Theorem` stamps — is answered
    // by making that audit mandatory and machine-readable, not by
    // making the compiler refuse.)
    let severity = match v.severity {
        // An unmet OBLIGATION (see `is_unmet_obligation`) is judged by
        // the audit, which owns CI's exit code; the compiler shows it
        // without refusing the build. A false CLAIM stays an error
        // here — a declaration the system can show is wrong is a
        // defect of the same order as a type error.
        KernelSeverity::Error if v.code.is_unmet_obligation() => Severity::Warning,
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
// The module-level flat walker is GONE (T0848): module surfaces come
// from `infer_module_summaries` — transitive, row-based, solved in
// the kernel. The per-item walker below remains for per-item
// `@arch_module` checks, whose scope is a single body by design.

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
        ExprKind::MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            // Dotted primitive calls ARE method calls in the AST
            // (receiver field-chain + method) — resolve them against
            // the ontology through the same resolver the row walker
            // uses. Type-aware receiver dispatch stays future work.
            if let Some(path) = method_call_dotted_path(receiver, method) {
                if let Some(cap) =
                    verum_kernel::arch_capability_inference::lookup_capability(&path)
                {
                    sink.insert(cap);
                }
            }
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

/// The bare name of an impl block's self type — `implement Point {`
/// and `implement Show for Point {` both yield `Point`; shapes without
/// a nameable head (tuples, references) yield None and the impl's
/// methods simply gain no qualified alias.
fn impl_self_type_name(impl_decl: &verum_ast::decl::ImplDecl) -> Option<String> {
    use verum_ast::decl::ImplKind;
    let ty = match &impl_decl.kind {
        ImplKind::Inherent(ty) => ty,
        ImplKind::Protocol { for_type, .. } => for_type,
    };
    match &ty.kind {
        verum_ast::ty::TypeKind::Path(path) => {
            path.segments.iter().rev().find_map(|seg| match seg {
                verum_ast::ty::PathSegment::Name(id) => {
                    Some(id.name.as_str().to_string())
                }
                _ => None,
            })
        }
        _ => None,
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
        // `core.net.tcp` parses as Field(Field(Path(core), net), tcp) —
        // the parser builds field chains, not multi-segment paths, for
        // dotted expressions. Without this arm every fully-qualified
        // primitive call was invisible to the ontology (the walker
        // matched a shape the parser never produces).
        ExprKind::Field { expr: base, field } => {
            let mut path = expr_to_dotted_path(base)?;
            path.push('.');
            path.push_str(field.name.as_str());
            Some(path)
        }
        _ => None,
    }
}

/// The dotted path of a METHOD call: `core.net.tcp.connect(...)` is
/// `MethodCall { receiver: core.net.tcp, method: connect }` in the
/// AST — the receiver chain plus the method name is the ontology key.
fn method_call_dotted_path(
    receiver: &verum_ast::expr::Expr,
    method: &verum_ast::Ident,
) -> Option<String> {
    let mut path = expr_to_dotted_path(receiver)?;
    path.push('.');
    path.push_str(method.name.as_str());
    Some(path)
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

// ============================================================================
// T0848 — row-based inference: per-function fact extraction
// ============================================================================
//
// The AST-aware HALF of the inference-first design
// (`docs/architecture/ats-v2-capability-rows.md` §3/§6): this layer
// only EXTRACTS facts — own ontology atoms, local call-graph edges,
// mixed capability-bearing parameters — and hands them to the
// kernel's AST-blind solver (`verum_kernel::arch_rows`), which owns
// the algebra and the fixpoint. Nothing here re-derives lattice law.

use verum_kernel::arch_rows::{FnFacts, ModuleFacts, ModuleSummaries};

/// Collect the call-relevant facts of one function body.
#[derive(Default)]
struct BodyFacts {
    /// Ontology atoms from fully-qualified primitive calls.
    atoms: std::collections::HashSet<verum_kernel::arch::Capability>,
    /// Bare names invoked as `name(...)` — candidate local callees
    /// AND candidate mixed-parameter invocations; the caller
    /// classifies them against the module's function set and the
    /// parameter list.
    bare_calls: std::collections::HashSet<String>,
    /// Dotted call paths the ontology did NOT resolve — candidate
    /// qualified local methods (`Point.new`), mounted/stdlib callees
    /// (`fs.open`, `core.fs.open`), and value-method dispatch
    /// (`x.push`); the caller classifies them, and what it cannot
    /// classify as a module edge it names value dispatch — the walker
    /// itself never swallows a dotted call again.
    dotted_calls: std::collections::HashSet<String>,
    /// Bare names passed as ARGUMENTS to calls — candidate
    /// hand-onward of capability-bearing parameters (`mix(r̄)`'s
    /// second clause).
    bare_args: std::collections::HashSet<String>,
}

/// Extract `ModuleFacts` for the kernel solver from a parsed module.
///
/// Classification rules (v1, module-local):
///   * A dotted path that resolves in the ontology → own atom.
///   * A bare call whose name is a module-local function → call-graph
///     edge (the solver joins the callee's summary transitively —
///     this is exactly what the flat walk could not see).
///   * A bare call/arg whose name is a FUNCTION-TYPED parameter of
///     the enclosing fn → mixed param (opens the summary, Rule G).
///   * Everything else stays untracked in v1 — cross-module summary
///     consumption is the next slice and lands as CITED module
///     interfaces, never as a silent widening.
/// Resolvers the extraction consults for CROSS-BOUNDARY facts:
/// mounted callees (qualified name → the callee module's solved row)
/// and protocol max-Shapes (protocol name → its Cited row). Both are
/// plain closures so `arch query` on a single file runs with empty
/// resolvers while the pipeline passes session-backed ones — one
/// extraction, two harnesses, no divergence.
pub(crate) struct ExtractionResolvers<'r> {
    /// Qualified mounted callee → solved summary row.
    pub imports: &'r dyn Fn(&str) -> Option<verum_kernel::arch_rows::Row>,
    /// Protocol name → declared `@max_shape` row (Cited).
    pub protocol_max_shape: &'r dyn Fn(&str) -> Option<verum_kernel::arch_rows::Row>,
}

impl Default for ExtractionResolvers<'static> {
    fn default() -> Self {
        ExtractionResolvers {
            imports: &|_| None,
            protocol_max_shape: &|_| None,
        }
    }
}

pub(crate) fn extract_module_facts(
    module: &Module,
    resolvers: &ExtractionResolvers<'_>,
) -> ModuleFacts {
    use verum_ast::decl::{FunctionBody, FunctionParamKind, ItemKind};

    let mut facts = ModuleFacts::default();

    // Mount map: bare name → qualified path, from every
    // `mount a.b.{x, y as z}` in the module. Globs are skipped —
    // their name set is unknowable here, and the no-silent-⊤ law
    // forbids guessing (a glob-mounted callee stays an unresolved
    // edge the report surfaces).
    let mount_map = build_mount_map(module);

    // Pass 1: the module-local function name set (free fns + impl
    // methods under their bare method name — local calls inside an
    // impl body use the bare name; qualified `Type.method` calls
    // also record under the qualified key).
    let mut dotted_by_fn: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut qualified_local_alias: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let collect_fn = |name: String,
                      fn_decl: &verum_ast::decl::FunctionDecl,
                      facts: &mut ModuleFacts,
                      dotted_by_fn: &mut std::collections::HashMap<String, Vec<String>>| {
            let mut body_facts = BodyFacts::default();
            if let Maybe::Some(body) = &fn_decl.body {
                match body {
                    FunctionBody::Block(block) => walk_block_for_facts(block, &mut body_facts),
                    FunctionBody::Expr(expr) => walk_expr_for_facts(expr, &mut body_facts),
                }
            }
            let mut own = verum_kernel::arch_rows::Row::computed(
                body_facts.atoms.iter().cloned(),
            );
            // Parameters, classified: a FUNCTION-typed parameter the
            // body invokes/hands on opens the summary (row variable,
            // Rule G); a PROTOCOL-typed parameter with a declared
            // max-Shape contributes that CITED row instead — bounded
            // polymorphism at the trait seam (§5b symmetry), and the
            // summary stays closed over it.
            let mut mixed_params: Vec<String> = Vec::new();
            for p in fn_decl.params.iter() {
                let FunctionParamKind::Regular { pattern, ty, .. } = &p.kind else {
                    continue;
                };
                let Some(pname) = pattern_bound_name(pattern) else {
                    continue;
                };
                let used = body_facts.bare_calls.contains(&pname)
                    || body_facts.bare_args.contains(&pname);
                if !used {
                    continue;
                }
                match &ty.kind {
                    verum_ast::ty::TypeKind::Function { .. } => {
                        mixed_params.push(pname);
                    }
                    verum_ast::ty::TypeKind::Path(path) => {
                        let tname = path
                            .segments
                            .iter()
                            .filter_map(|s| match s {
                                verum_ast::ty::PathSegment::Name(id) => {
                                    Some(id.name.as_str())
                                }
                                _ => None,
                            })
                            .next_back()
                            .unwrap_or("");
                        if let Some(max) = (resolvers.protocol_max_shape)(tname) {
                            own.join(&max);
                        }
                    }
                    _ => {}
                }
            }
            // Mounted callees: resolve bare → qualified through the
            // mount map, then qualified → summary through the imports
            // resolver. Resolved rows join OWN (their provenance is
            // whatever the callee's summary carries); unresolved
            // qualified names go to callees so the solver SURFACES
            // them instead of anyone guessing.
            let mut callees: Vec<String> = Vec::new();
            for bare in body_facts.bare_calls.iter() {
                if let Some(qualified) = mount_map.get(bare) {
                    match (resolvers.imports)(qualified) {
                        Some(row) => own.join(&row),
                        None => callees.push(qualified.clone()),
                    }
                } else {
                    callees.push(bare.clone());
                }
            }
            let mut dotted: Vec<String> =
                body_facts.dotted_calls.iter().cloned().collect();
            dotted.sort();
            dotted_by_fn.insert(name.clone(), dotted);
            facts.functions.insert(
                name,
                FnFacts {
                    own,
                    callees,
                    mixed_params,
                },
            );
        };

    for item in &module.items {
        match &item.kind {
            ItemKind::Function(fn_decl) => {
                collect_fn(
                    fn_decl.name.name.to_string(),
                    fn_decl,
                    &mut facts,
                    &mut dotted_by_fn,
                );
            }
            ItemKind::Impl(impl_decl) => {
                // The self-type's bare name, so `Type.method(...)` call
                // sites link to the SAME summary the bare method name
                // carries (one summary, two spellings — an alias, not
                // a second node).
                let self_ty_name = impl_self_type_name(impl_decl);
                for impl_item in impl_decl.items.iter() {
                    if let verum_ast::decl::ImplItemKind::Function(f) = &impl_item.kind {
                        let bare = f.name.name.to_string();
                        if let Some(ty) = &self_ty_name {
                            qualified_local_alias
                                .insert(format!("{ty}.{bare}"), bare.clone());
                        }
                        collect_fn(bare, f, &mut facts, &mut dotted_by_fn);
                    }
                }
            }
            _ => {}
        }
    }

    // Pass 2: keep as call-graph edges only names that ARE
    // module-local functions and are not this fn's own mixed params
    // (a call through a parameter is polymorphism, not an edge).
    let local_names: std::collections::HashSet<String> =
        facts.functions.keys().cloned().collect();
    // Dotted calls, classified now that the local name set exists:
    //   * qualified local method (`Point.new`)      → call-graph edge;
    //   * mount-expanded / stdlib-rooted callee     → resolved row, or
    //     an UNRESOLVED edge the solver surfaces (no-silent-⊤);
    //   * anything else is value-method dispatch (`x.push(1)`) — the
    //     carry(T) law at the type seam governs it, not a module
    //     edge; typed receiver dispatch is future work, never a guess.
    for (name, fn_facts) in facts.functions.iter_mut() {
        for dotted in dotted_by_fn.get(name).into_iter().flatten() {
            let Some((first, rest)) = dotted.split_once('.') else {
                continue;
            };
            let qualified = match mount_map.get(first) {
                Some(prefix) => format!("{prefix}.{rest}"),
                None => dotted.clone(),
            };
            if local_names.contains(&qualified) {
                fn_facts.callees.push(qualified);
            } else if let Some(bare) = qualified_local_alias.get(&qualified) {
                fn_facts.callees.push(bare.clone());
            } else if let Some(row) = (resolvers.imports)(&qualified) {
                fn_facts.own.join(&row);
            } else if mount_map.contains_key(first) || qualified.starts_with("core.") {
                fn_facts.callees.push(qualified);
            }
        }
    }
    for fn_facts in facts.functions.values_mut() {
        let mixed: std::collections::HashSet<&String> =
            fn_facts.mixed_params.iter().collect();
        fn_facts.callees.retain(|c| {
            // Keep: local edges (the graph) and UNRESOLVED mounted
            // callees (qualified, dotted — the solver returns them as
            // unresolved so the report surfaces them). Drop: bare
            // names that are neither local nor mounted (locals of
            // closures, builtins) and calls through mixed params
            // (polymorphism, not edges).
            (local_names.contains(c) || c.contains('.')) && !mixed.contains(c)
        });
    }

    facts
}

/// Bare name → qualified path for every non-glob mount of the module.
fn build_mount_map(module: &Module) -> std::collections::BTreeMap<String, String> {
    use verum_ast::decl::{ItemKind, MountTreeKind};
    fn path_to_string(p: &verum_ast::ty::Path) -> String {
        p.segments
            .iter()
            .filter_map(|s| match s {
                verum_ast::ty::PathSegment::Name(id) => Some(id.name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(".")
    }
    fn walk_tree(
        tree: &verum_ast::decl::MountTree,
        prefix: Option<&str>,
        out: &mut std::collections::BTreeMap<String, String>,
    ) {
        match &tree.kind {
            MountTreeKind::Path(p) => {
                let full = match prefix {
                    Some(pre) => format!("{pre}.{}", path_to_string(p)),
                    None => path_to_string(p),
                };
                let bare = match &tree.alias {
                    Maybe::Some(a) => a.name.to_string(),
                    Maybe::None => full
                        .rsplit('.')
                        .next()
                        .unwrap_or(&full)
                        .to_string(),
                };
                out.insert(bare, full);
            }
            MountTreeKind::Nested { prefix: p, trees } => {
                let pre = match prefix {
                    Some(outer) => format!("{outer}.{}", path_to_string(p)),
                    None => path_to_string(p),
                };
                for t in trees.iter() {
                    walk_tree(t, Some(&pre), out);
                }
            }
            // Globs and file-relative mounts: name set unknowable
            // here — deliberately unmapped (no-silent-⊤).
            _ => {}
        }
    }
    let mut out = std::collections::BTreeMap::new();
    for item in &module.items {
        if let ItemKind::Mount(m) = &item.kind {
            walk_tree(&m.tree, None, &mut out);
        }
    }
    out
}

/// Collect `@max_shape(requires: [...])`-style declarations on the
/// module's PROTOCOL types: protocol name → its Cited row. The pin
/// grammar is the same capability list the `@arch_module` parser
/// reads, reused verbatim (one parser, two sites).
pub(crate) fn collect_protocol_max_shapes(
    module: &Module,
) -> std::collections::BTreeMap<String, verum_kernel::arch_rows::Row> {
    use verum_ast::decl::ItemKind;
    let mut out = std::collections::BTreeMap::new();
    for item in &module.items {
        // `type P is protocol {...}` may arrive as ItemKind::Type OR
        // the standalone ItemKind::Protocol — one collector, both
        // spellings.
        // The attribute rides on the DECL (TypeDecl.attributes), not
        // on the item — the parser attaches type-level attributes to
        // the declaration node.
        let (proto_name, decl_attrs): (String, Option<&List<verum_ast::attr::Attribute>>) =
            match &item.kind {
                ItemKind::Type(t) => (t.name.name.to_string(), Some(&t.attributes)),
                // Standalone ProtocolDecl carries no decl-attr list —
                // its attributes live on the item.
                ItemKind::Protocol(pd) => (pd.name.name.to_string(), None),
                _ => continue,
            };
        for attr in decl_attrs
            .into_iter()
            .flat_map(|l| l.iter())
            .chain(item.attributes.iter())
        {
            if attr.name.as_str() != "max_shape" {
                continue;
            }
            let args = match &attr.args {
                Maybe::Some(a) => a.as_slice(),
                Maybe::None => &[],
            };
            if let Ok(shape) = verum_kernel::arch_parse::parse_arch_module(args) {
                let row = verum_kernel::arch_rows::Row::cited(
                    shape.requires.iter().chain(shape.exposes.iter()).cloned(),
                    &format!("protocol {proto_name} @max_shape"),
                );
                out.insert(proto_name.clone(), row);
            }
        }
    }
    out
}

/// The single bound name of an irrefutable parameter pattern, when it
/// has one (v1 skips destructuring patterns — a destructured fn-typed
/// parameter is rare and lands with cross-module summaries).
fn pattern_bound_name(pattern: &verum_ast::Pattern) -> Option<String> {
    use verum_ast::PatternKind;
    match &pattern.kind {
        PatternKind::Ident { name, .. } => Some(name.name.to_string()),
        _ => None,
    }
}

fn walk_block_for_facts(block: &verum_ast::Block, sink: &mut BodyFacts) {
    for stmt in block.stmts.iter() {
        walk_stmt_for_facts(stmt, sink);
    }
    if let Maybe::Some(tail) = &block.expr {
        walk_expr_for_facts(tail, sink);
    }
}

fn walk_stmt_for_facts(stmt: &verum_ast::stmt::Stmt, sink: &mut BodyFacts) {
    use verum_ast::stmt::StmtKind;
    match &stmt.kind {
        StmtKind::Expr { expr, .. } => walk_expr_for_facts(expr, sink),
        StmtKind::Let { value, .. } => {
            if let Maybe::Some(init) = value {
                walk_expr_for_facts(init, sink);
            }
        }
        StmtKind::LetElse { value, .. } => walk_expr_for_facts(value, sink),
        StmtKind::Defer(expr) | StmtKind::Errdefer(expr) => walk_expr_for_facts(expr, sink),
        StmtKind::Provide { value, .. } | StmtKind::ProvideScope { value, .. } => {
            walk_expr_for_facts(value, sink);
        }
        _ => {}
    }
}

fn walk_expr_for_facts(expr: &verum_ast::expr::Expr, sink: &mut BodyFacts) {
    use verum_ast::expr::ExprKind;
    match &expr.kind {
        ExprKind::Call { func, args, .. } => {
            if let Some(path) = expr_to_dotted_path(func) {
                if let Some(cap) =
                    verum_kernel::arch_capability_inference::lookup_capability(&path)
                {
                    sink.atoms.insert(cap);
                } else if !path.contains('.') {
                    sink.bare_calls.insert(path);
                } else {
                    sink.dotted_calls.insert(path);
                }
            }
            walk_expr_for_facts(func, sink);
            for a in args.iter() {
                if let Some(p) = expr_to_dotted_path(a) {
                    if !p.contains('.') {
                        sink.bare_args.insert(p);
                    }
                }
                walk_expr_for_facts(a, sink);
            }
        }
        ExprKind::MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            if let Some(path) = method_call_dotted_path(receiver, method) {
                if let Some(cap) =
                    verum_kernel::arch_capability_inference::lookup_capability(&path)
                {
                    sink.atoms.insert(cap);
                } else {
                    sink.dotted_calls.insert(path);
                }
            }
            walk_expr_for_facts(receiver, sink);
            for a in args.iter() {
                if let Some(p) = expr_to_dotted_path(a) {
                    if !p.contains('.') {
                        sink.bare_args.insert(p);
                    }
                }
                walk_expr_for_facts(a, sink);
            }
        }
        ExprKind::Block(block) => walk_block_for_facts(block, sink),
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            walk_block_for_facts(then_branch, sink);
            if let Maybe::Some(else_b) = else_branch {
                walk_expr_for_facts(else_b, sink);
            }
        }
        ExprKind::Match { expr: scrut, arms } => {
            walk_expr_for_facts(scrut, sink);
            for arm in arms.iter() {
                walk_expr_for_facts(&arm.body, sink);
            }
        }
        ExprKind::While {
            condition, body, ..
        } => {
            walk_expr_for_facts(condition, sink);
            walk_block_for_facts(body, sink);
        }
        ExprKind::For { iter, body, .. } => {
            walk_expr_for_facts(iter, sink);
            walk_block_for_facts(body, sink);
        }
        ExprKind::Loop { body, .. } => walk_block_for_facts(body, sink),
        ExprKind::Binary { left, right, .. } => {
            walk_expr_for_facts(left, sink);
            walk_expr_for_facts(right, sink);
        }
        ExprKind::Unary { expr: inner, .. } => walk_expr_for_facts(inner, sink),
        ExprKind::Field { expr: inner, .. }
        | ExprKind::OptionalChain { expr: inner, .. }
        | ExprKind::TupleIndex { expr: inner, .. } => walk_expr_for_facts(inner, sink),
        ExprKind::Index { expr: e, index } => {
            walk_expr_for_facts(e, sink);
            walk_expr_for_facts(index, sink);
        }
        ExprKind::Tuple(items) => {
            for e in items.iter() {
                walk_expr_for_facts(e, sink);
            }
        }
        _ => {}
    }
}

/// Row-based module inference: extract facts, solve, return the
/// module surface as the flat capability list the existing phase
/// plumbing consumes — TRANSITIVE now (a helper's `Network` reaches
/// its caller's surface through the summary join), where the flat
/// walk saw only direct calls.
pub(crate) fn infer_module_summaries(module: &Module) -> ModuleSummaries {
    // Single-module harness: local protocol max-Shapes resolve; the
    // import resolver is empty (mounted callees surface as
    // unresolved). The PIPELINE harness threads session-backed
    // resolvers through `infer_module_summaries_with`.
    let local_protocols = collect_protocol_max_shapes(module);
    let resolvers = ExtractionResolvers {
        imports: &|_| None,
        protocol_max_shape: &|name| local_protocols.get(name).cloned(),
    };
    extract_module_facts(module, &resolvers).solve()
}

/// Pipeline harness: resolvers backed by the compilation session's
/// cross-module registries.
pub(crate) fn infer_module_summaries_with(
    module: &Module,
    resolvers: &ExtractionResolvers<'_>,
) -> ModuleSummaries {
    extract_module_facts(module, resolvers).solve()
}

// ============================================================================
// Public query surface (consumed by crate::arch_query — the CLI tool)
// ============================================================================

/// Public entry for `verum arch query`: the SAME extraction+solve the
/// phase runs — one derivation, two consumers (compiler diagnostics
/// and the agent-facing tool must never disagree about a surface).
pub fn infer_summaries_for_query(module: &Module) -> ModuleSummaries {
    infer_module_summaries(module)
}

/// The module's PINNED capability row, when the module-level
/// `@arch_module` declares one: `requires ∪ exposes`, provenance
/// `Cited` (a pin is intent taken on the author's authority — the
/// inference is what is `Computed`).
pub fn pinned_capabilities_of_module(
    module: &Module,
) -> Option<verum_kernel::arch_rows::Row> {
    let mut args: Option<&[verum_ast::expr::Expr]> = None;
    let attr_sources = std::iter::once(&module.attributes)
        .chain(module.items.iter().map(|i| &i.attributes));
    for attrs in attr_sources {
        for attr in attrs.iter() {
            if attr.name.as_str() == "arch_module" {
                args = Some(match &attr.args {
                    Maybe::Some(a) => a.as_slice(),
                    Maybe::None => &[],
                });
            }
        }
        if args.is_some() {
            break;
        }
    }
    let shape = verum_kernel::arch_parse::parse_arch_module(args?).ok()?;
    let row = verum_kernel::arch_rows::Row::cited(
        shape
            .requires
            .iter()
            .chain(shape.exposes.iter())
            .cloned(),
        "@arch_module pin",
    );
    // A pin with zero capabilities is still a pin (an explicitly
    // empty surface is a claim worth judging against).
    Some(row)
}
