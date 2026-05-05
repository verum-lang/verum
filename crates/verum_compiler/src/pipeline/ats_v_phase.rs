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
use verum_kernel::arch_anti_pattern::{AntiPatternViolation, Severity as KernelSeverity};
use verum_kernel::arch_phase::{run_arch_phase_one, ModuleArchResult};

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    /// ATS-V phase — runs the architectural-type checker over every
    /// `@arch_module(...)` declaration in the module.  Emits one
    /// diagnostic per violation; never fails the build directly
    /// (the diagnostic stream's `abort_if_errors` handles aggregation).
    pub(super) fn phase_ats_v(&self, module: &Module) -> Result<()> {
        let mut total_violations = 0usize;

        // 1. Module-level @arch_module(...) — the primary surface.
        if let Some(result) = run_arch_phase_for_attrs(&module.attributes, "<module>") {
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
            if let Some(result) = run_arch_phase_for_attrs(&item.attributes, &item_name) {
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

/// Walk an attribute list looking for `@arch_module(...)`.  Returns
/// the kernel-side phase result (or `None` if no annotation found).
fn run_arch_phase_for_attrs(
    attrs: &List<verum_ast::attr::Attribute>,
    module_name: &str,
) -> Option<ModuleArchResult> {
    for attr in attrs.iter() {
        if attr.name.as_str() != "arch_module" {
            continue;
        }
        // Extract the named-arg expressions.  An empty arg list
        // is valid — it fires the "minimal shape" code path on
        // the kernel side: `run_arch_phase_one` returns no
        // parse errors and runs the canonical 32 anti-pattern
        // checks against `Shape::default_for_unannotated()`,
        // which passes every check vacuously.
        let args_slice: &[verum_ast::expr::Expr] = match &attr.args {
            Maybe::Some(args) => args.as_slice(),
            Maybe::None => &[],
        };
        return Some(run_arch_phase_one(module_name.to_string(), args_slice));
    }
    None
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::attr::Attribute;
    use verum_ast::span::Span;
    use verum_common::Text;

    fn empty_attrs() -> List<Attribute> {
        List::default()
    }

    fn attrs_with_arch_module(args: Maybe<List<verum_ast::expr::Expr>>) -> List<Attribute> {
        let attr = Attribute {
            name: Text::from("arch_module"),
            args,
            span: Span::dummy(),
        };
        let mut list: List<Attribute> = List::default();
        list.push(attr);
        list
    }

    #[test]
    fn no_arch_module_attr_returns_none() {
        // Architectural pin: phase walks attrs but skips modules
        // without `@arch_module(...)` per spec §17.5 backward-compat.
        // Returns `None` so the caller doesn't emit any diagnostic.
        assert!(run_arch_phase_for_attrs(&empty_attrs(), "any").is_none());
    }

    #[test]
    fn arch_module_attr_invokes_kernel_phase() {
        // Pin: an `@arch_module(...)` attribute drives the kernel-side
        // phase even with empty args.  Empty args → kernel returns a
        // clean ModuleArchResult with no parse errors and no
        // violations against the default shape.
        let result = run_arch_phase_for_attrs(&attrs_with_arch_module(Maybe::None), "test_mod");
        assert!(result.is_some(), "phase must fire when @arch_module is present");
        let r = result.unwrap();
        assert!(
            r.parse_errors.is_empty(),
            "empty-args case must produce no parse errors"
        );
        // Default-shape sanity: no violations under the default shape.
        assert!(r.violations.is_empty(), "default shape must pass all checks");
        assert_eq!(r.module_name, "test_mod");
    }

    #[test]
    fn run_arch_phase_for_attrs_walks_first_arch_module_only() {
        // Pin: when an attribute list contains a non-arch_module
        // attribute first and then an arch_module, the walker still
        // finds it (returning Some) so the diagnostic emit path runs.
        let mut list: List<Attribute> = List::default();
        list.push(Attribute::simple(Text::from("derive"), Span::dummy()));
        list.push(Attribute {
            name: Text::from("arch_module"),
            args: Maybe::None,
            span: Span::dummy(),
        });
        list.push(Attribute::simple(Text::from("inline"), Span::dummy()));
        let result = run_arch_phase_for_attrs(&list, "test_mod");
        assert!(result.is_some());
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
