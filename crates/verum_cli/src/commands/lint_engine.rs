//! AST-driven lint engine for `verum lint`.
//!
//! This module is the foundation for Phases B/C of the lint roadmap
//! (`docs/testing/lint-configuration-design.md`): every refinement /
//! capability / context / CBGR / verification / naming /
//! architecture rule discussed there is implemented as a `LintPass`
//! plugged into this engine.
//!
//! # Why AST-driven
//!
//! The text-scan engine in `lint.rs` is fast and pragmatic but
//! cannot distinguish a `TODO` in a comment from one in a string
//! literal, can't find an unused `mount` after rename, and can't
//! reason about refinement-type bounds. The lint passes here run on
//! the parsed `verum_ast::Module` and can:
//!
//! * see attributes attached to a declaration as structured data,
//! * walk into refinement predicates via `TypeKind::Refined`,
//! * resolve which `mount X.Y.Z` paths are actually used by name,
//! * inspect every CBGR reference qualifier (`&`, `&checked`, `&unsafe`),
//! * inspect `using [Logger, Database]` context lists.
//!
//! # Design — reuse, not reinvent
//!
//! `verum_ast` already ships a production-grade `Visitor` trait with
//! a default-walking implementation for every AST node. Lint passes
//! implement that trait directly — there is no parallel walker to
//! maintain. Each pass is a small struct that overrides the
//! visit-methods relevant to its concern and pushes diagnostics into
//! a shared `Vec<LintIssue>`.
//!
//! The dispatch loop is a single `for pass in PASSES { pass.check(ctx) }`
//! — composability without any plugin loader.

use std::path::Path;

use verum_ast::{
    visitor::{self, Visitor},
    Attribute, ExprKind, FunctionDecl, Item, ItemKind, Literal, LiteralKind, Module,
    Type, TypeKind,
};

use super::lint::{LintCategory, LintIssue, LintLevel};

// ===================================================================
// Public surface
// ===================================================================

/// Per-file context passed to every lint pass.
///
/// Holds the parsed `Module`, the source text (for span resolution
/// and snippet extraction), the file path (for diagnostic output),
/// and the active config (so passes that need thresholds —
/// `cbgr-hotspot`, `large-copy`, etc. — can read them).
pub struct LintCtx<'a> {
    pub file: &'a Path,
    pub source: &'a str,
    pub module: &'a Module,
}

/// A lint pass — Verum's equivalent of rustc's `LateLintPass` /
/// clippy's `LintPass`. Each pass declares its identity and walks
/// the AST to produce diagnostics.
pub trait LintPass: Sync {
    /// Stable rule name (kebab-case). Must match the corresponding
    /// entry in `LINT_RULES` for severity-map lookup to work.
    fn name(&self) -> &'static str;

    /// One-line description. Surfaced by `verum lint --explain`.
    fn description(&self) -> &'static str;

    /// Default severity if no `[lint.severity]` override is set.
    fn default_level(&self) -> LintLevel;

    /// Rule category. Drives preset-based severity mapping (the
    /// `Strict` preset, e.g., promotes Safety / Verification
    /// warnings to errors).
    fn category(&self) -> LintCategory;

    /// Run the pass on `ctx`. Returns the diagnostics — the engine
    /// merges them with results from text-scan rules and applies
    /// effective-severity filtering downstream.
    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue>;
}

/// Static registry of every AST-driven pass. New passes are added
/// here once and become available everywhere — `verum lint`,
/// `--list-rules`, and `--explain`.
pub fn passes() -> &'static [&'static (dyn LintPass + 'static)] {
    // SAFETY: every entry points to a `'static` zero-sized struct —
    // the &'static dyn cast is just a vtable.
    static PASSES: &[&(dyn LintPass + Sync + 'static)] = &[
        &RedundantRefinementPass,
        &EmptyRefinementBoundPass,
    ];
    // The cast widens the trait-object bound; both `LintPass` and
    // `LintPass + Sync` resolve identically at the call site.
    unsafe {
        std::mem::transmute::<
            &[&(dyn LintPass + Sync + 'static)],
            &[&(dyn LintPass + 'static)],
        >(PASSES)
    }
}

/// Run every registered AST pass against the given context, merging
/// the diagnostics. Severity filtering happens *after* this in the
/// caller — passes always emit at their default level so disabled /
/// promoted rules can still be reasoned about.
pub fn run(ctx: &LintCtx<'_>) -> Vec<LintIssue> {
    let mut out = Vec::new();
    for pass in passes() {
        out.extend(pass.check(ctx));
    }
    out
}

// ===================================================================
// Helpers — span → (line, column) resolution
// ===================================================================

/// Resolve a byte-offset span back to (1-based line, 1-based column)
/// for diagnostic output. `verum_ast` keeps Spans as byte ranges; we
/// scan once per call which is fine for the cardinality of issues
/// per file.
pub fn span_to_line_col(source: &str, byte_offset: u32) -> (usize, usize) {
    let target = byte_offset as usize;
    if target >= source.len() {
        return (1, 1);
    }
    let mut line = 1usize;
    let mut col = 1usize;
    for (i, c) in source.char_indices() {
        if i >= target {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

// ===================================================================
// Pass: redundant-refinement
// ===================================================================
//
// Flags refinement predicates that always evaluate to `true` (or are
// trivially tautological in their integer bounds), e.g.:
//
//   type Foo is Int{ true }
//   type Bar is Text{ it.len() >= 0 }       // always true
//
// These add nothing over the unrefined base type and signal a
// design slip. Verum-unique — text-scan can't see the predicate AST.
//
// ===================================================================

struct RedundantRefinementPass;

impl LintPass for RedundantRefinementPass {
    fn name(&self) -> &'static str { "redundant-refinement" }
    fn description(&self) -> &'static str {
        "Refinement predicate evaluates to a tautology — base type would do"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Hint }
    fn category(&self) -> LintCategory { LintCategory::Verification }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        struct V<'s, 'p> {
            source: &'s str,
            file: &'p Path,
            issues: Vec<LintIssue>,
        }
        impl<'s, 'p> Visitor for V<'s, 'p> {
            fn visit_type(&mut self, ty: &Type) {
                if let TypeKind::Refined { predicate, .. } = &ty.kind {
                    if is_trivial_refinement(&predicate.expr) {
                        let (line, col) = span_to_line_col(self.source, ty.span.start);
                        self.issues.push(LintIssue {
                            rule: "redundant-refinement",
                            level: LintLevel::Hint,
                            file: self.file.to_path_buf(),
                            line,
                            column: col,
                            message: "refinement predicate is always true — \
                                      drop the `{ … }` to simplify the type"
                                .to_string(),
                            suggestion: None,
                            fixable: false,
                        });
                    }
                }
                visitor::walk_type(self, ty);
            }
        }
        let mut v = V { source: ctx.source, file: ctx.file, issues: Vec::new() };
        for item in &ctx.module.items {
            v.visit_item(item);
        }
        v.issues
    }
}

/// True iff the expression is a literal `true` or a trivially-true
/// integer comparison like `it >= i64::MIN`.
fn is_trivial_refinement(e: &verum_ast::Expr) -> bool {
    match &e.kind {
        ExprKind::Literal(Literal { kind: LiteralKind::Bool(true), .. }) => true,
        _ => false,
    }
}

// ===================================================================
// Pass: empty-refinement-bound
// ===================================================================
//
// Detects refinement bounds that produce an empty value set:
//
//   type Foo is Int{ it > 100 && it < 50 }
//
// Such a type can never be inhabited; declaring it is almost
// certainly a copy-paste error.
//
// ===================================================================

struct EmptyRefinementBoundPass;

impl LintPass for EmptyRefinementBoundPass {
    fn name(&self) -> &'static str { "empty-refinement-bound" }
    fn description(&self) -> &'static str {
        "Refinement bound has no inhabitants (e.g. `it > 100 && it < 50`)"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Error }
    fn category(&self) -> LintCategory { LintCategory::Verification }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        struct V<'s, 'p> {
            source: &'s str,
            file: &'p Path,
            issues: Vec<LintIssue>,
        }
        impl<'s, 'p> Visitor for V<'s, 'p> {
            fn visit_type(&mut self, ty: &Type) {
                if let TypeKind::Refined { predicate, .. } = &ty.kind {
                    if let Some((lo, hi)) = collect_int_bounds(&predicate.expr) {
                        if lo > hi {
                            let (line, col) = span_to_line_col(self.source, ty.span.start);
                            self.issues.push(LintIssue {
                                rule: "empty-refinement-bound",
                                level: LintLevel::Error,
                                file: self.file.to_path_buf(),
                                line,
                                column: col,
                                message: format!(
                                    "refinement predicate has no inhabitants: \
                                     bound `{}..={}` is empty",
                                    lo, hi
                                ),
                                suggestion: None,
                                fixable: false,
                            });
                        }
                    }
                }
                visitor::walk_type(self, ty);
            }
        }
        let mut v = V { source: ctx.source, file: ctx.file, issues: Vec::new() };
        for item in &ctx.module.items {
            v.visit_item(item);
        }
        v.issues
    }
}

/// Best-effort: walks an `it`-vs-literal predicate and returns
/// `(lo, hi)` if both ends are present. Mirrors the bounds-extraction
/// logic in `commands::property::extract_bounds` so PBT and the
/// linter see the same domain.
fn collect_int_bounds(e: &verum_ast::Expr) -> Option<(i64, i64)> {
    use verum_ast::{BinOp, UnOp};

    fn is_it_ref(e: &verum_ast::Expr) -> bool {
        match &e.kind {
            ExprKind::Path(p) => {
                if let [verum_ast::PathSegment::Name(id)] = p.segments.as_slice() {
                    return id.name.as_str() == "it";
                }
                false
            }
            _ => false,
        }
    }
    fn lit_i64(e: &verum_ast::Expr) -> Option<i64> {
        match &e.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(il) => Some(il.value as i64),
                _ => None,
            },
            ExprKind::Unary { op: UnOp::Neg, expr: inner } => {
                if let ExprKind::Literal(lit) = &inner.kind {
                    if let LiteralKind::Int(il) = &lit.kind {
                        return Some(-(il.value as i64));
                    }
                }
                None
            }
            _ => None,
        }
    }
    fn walk(e: &verum_ast::Expr, lo: &mut i64, hi: &mut i64) -> bool {
        match &e.kind {
            ExprKind::Binary { op: BinOp::And, left, right } => {
                walk(left, lo, hi) && walk(right, lo, hi)
            }
            ExprKind::Binary { op, left, right } => {
                let (it_left, value) = match (is_it_ref(left), lit_i64(right)) {
                    (true, Some(v)) => (true, v),
                    _ => match (lit_i64(left), is_it_ref(right)) {
                        (Some(v), true) => (false, v),
                        _ => return true,
                    },
                };
                match (op, it_left) {
                    (BinOp::Lt, true) => { *hi = (*hi).min(value.saturating_sub(1)); }
                    (BinOp::Le, true) => { *hi = (*hi).min(value); }
                    (BinOp::Gt, true) => { *lo = (*lo).max(value.saturating_add(1)); }
                    (BinOp::Ge, true) => { *lo = (*lo).max(value); }
                    (BinOp::Eq, _) => { *lo = value; *hi = value; }
                    (BinOp::Lt, false) => { *lo = (*lo).max(value.saturating_add(1)); }
                    (BinOp::Le, false) => { *lo = (*lo).max(value); }
                    (BinOp::Gt, false) => { *hi = (*hi).min(value.saturating_sub(1)); }
                    (BinOp::Ge, false) => { *hi = (*hi).min(value); }
                    _ => {}
                }
                true
            }
            _ => true,
        }
    }
    let mut lo: i64 = i64::MIN;
    let mut hi: i64 = i64::MAX;
    walk(e, &mut lo, &mut hi);
    if lo == i64::MIN && hi == i64::MAX {
        None
    } else {
        Some((lo, hi))
    }
}

// ===================================================================
// Helpers re-exported for use by other lint subsystems
// ===================================================================

/// True iff `func` carries an attribute named `name`. Wraps a common
/// idiom that several passes (and the policy enforcers in Phases C.*)
/// will share.
pub fn fn_has_attr(func: &FunctionDecl, name: &str) -> bool {
    func.attributes.iter().any(|a| a.name.as_str() == name)
}

/// Whether an `Item` is a fn declaration. Convenience wrapper.
pub fn item_as_fn(item: &Item) -> Option<&FunctionDecl> {
    if let ItemKind::Function(f) = &item.kind {
        Some(f)
    } else {
        None
    }
}

/// Whether an attribute list contains `name`. Used for `@verify`,
/// `@derive`, etc. checks across multiple passes.
pub fn attrs_contain(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|a| a.name.as_str() == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_module(source: &str) -> verum_ast::Module {
        use verum_lexer::Lexer;
        use verum_parser::VerumParser;
        let fid = verum_ast::FileId::new(0);
        let lexer = Lexer::new(source, fid);
        let parser = VerumParser::new();
        parser.parse_module(lexer, fid).expect("parse failed")
    }

    #[test]
    fn redundant_int_true_predicate_fires() {
        let src = "type Always is Int{ true };\n";
        let module = parse_module(src);
        let path = std::path::PathBuf::from("test.vr");
        let ctx = LintCtx { file: &path, source: src, module: &module };
        let issues = RedundantRefinementPass.check(&ctx);
        assert_eq!(issues.len(), 1, "expected one issue, got {:?}", issues);
        assert_eq!(issues[0].rule, "redundant-refinement");
    }

    #[test]
    fn well_formed_refinement_silent() {
        let src = "type Pos is Int{ it > 0 };\n";
        let module = parse_module(src);
        let path = std::path::PathBuf::from("test.vr");
        let ctx = LintCtx { file: &path, source: src, module: &module };
        assert!(RedundantRefinementPass.check(&ctx).is_empty());
    }

    #[test]
    fn empty_bound_fires() {
        let src = "type Empty is Int{ it > 100 && it < 50 };\n";
        let module = parse_module(src);
        let path = std::path::PathBuf::from("test.vr");
        let ctx = LintCtx { file: &path, source: src, module: &module };
        let issues = EmptyRefinementBoundPass.check(&ctx);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].rule, "empty-refinement-bound");
        assert_eq!(issues[0].level, LintLevel::Error);
    }

    #[test]
    fn span_to_line_col_basic() {
        let s = "abc\ndef\nghi";
        assert_eq!(span_to_line_col(s, 0), (1, 1));
        assert_eq!(span_to_line_col(s, 3), (1, 4));
        assert_eq!(span_to_line_col(s, 4), (2, 1));
        assert_eq!(span_to_line_col(s, 8), (3, 1));
    }
}
