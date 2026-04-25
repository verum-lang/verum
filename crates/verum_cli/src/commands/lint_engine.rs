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

use super::lint::{LintCategory, LintConfig, LintIssue, LintLevel};

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
    /// Resolved config for this run. Passes that need thresholds,
    /// exemptions, or feature toggles read them via
    /// `cfg.rule_config::<T>("rule-name")`. None when the run is
    /// invoked without a project config (single-file mode).
    pub config: Option<&'a LintConfig>,
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
        &NamingConventionPass,
        &UnrefinedPublicIntPass,
        &VerifyImpliedByRefinementPass,
        &PublicMustHaveVerifyPass,
        &ForbiddenContextPass,
        &ArchitectureViolationPass,
        &CbgrBudgetExceededPass,
        &MaxLineLengthPass,
        &MaxFnLinesPass,
        &MaxFnParamsPass,
        &MaxMatchArmsPass,
        &PublicMustHaveDocPass,
        &UnsafeWithoutCapabilityPass,
        &FfiWithoutCapabilityPass,
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

// ===================================================================
// Pass: naming-convention
// ===================================================================
//
// Per-construct casing enforcement, configured via `[lint.naming]`:
//
//     [lint.naming]
//     fn        = "snake_case"
//     type      = "PascalCase"
//     const     = "SCREAMING_SNAKE_CASE"
//     variant   = "PascalCase"
//
//     [lint.naming.exempt]
//     fn   = ["__init", "drop_impl"]
//     type = ["I32", "F64"]
//
// Fires per declaration whose identifier doesn't match the
// corresponding convention. Convention names are validated at
// config-load time — typos surface at `verum lint --validate-config`.
// Defaults match Verum's documented style guide
// (`docs/guides/style-guide.md`).
//
// ===================================================================

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
struct NamingConfig {
    #[serde(rename = "fn")]
    fn_case: String,
    #[serde(rename = "type")]
    type_case: String,
    #[serde(rename = "const")]
    const_case: String,
    variant: String,
    field: String,
    module: String,
    generic: String,
    exempt: NamingExempt,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
struct NamingExempt {
    #[serde(rename = "fn")]
    fn_names: Vec<String>,
    #[serde(rename = "type")]
    type_names: Vec<String>,
    #[serde(rename = "const")]
    const_names: Vec<String>,
    variant: Vec<String>,
    field: Vec<String>,
    module: Vec<String>,
    generic: Vec<String>,
}

impl Default for NamingConfig {
    fn default() -> Self {
        Self {
            fn_case: "snake_case".into(),
            type_case: "PascalCase".into(),
            const_case: "SCREAMING_SNAKE_CASE".into(),
            variant: "PascalCase".into(),
            field: "snake_case".into(),
            module: "snake_case".into(),
            generic: "PascalCase".into(),
            exempt: NamingExempt::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Casing {
    SnakeCase,
    KebabCase,
    PascalCase,
    CamelCase,
    ScreamingSnakeCase,
    Lowercase,
    Uppercase,
}

impl Casing {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "snake_case" => Some(Self::SnakeCase),
            "kebab-case" => Some(Self::KebabCase),
            "PascalCase" => Some(Self::PascalCase),
            "camelCase" => Some(Self::CamelCase),
            "SCREAMING_SNAKE_CASE" => Some(Self::ScreamingSnakeCase),
            "lowercase" => Some(Self::Lowercase),
            "UPPERCASE" => Some(Self::Uppercase),
            _ => None,
        }
    }

    fn matches(self, ident: &str) -> bool {
        if ident.is_empty() {
            return true;
        }
        match self {
            Self::SnakeCase => ident
                .chars()
                .all(|c| c == '_' || c.is_ascii_lowercase() || c.is_ascii_digit()),
            Self::KebabCase => ident
                .chars()
                .all(|c| c == '-' || c.is_ascii_lowercase() || c.is_ascii_digit()),
            Self::PascalCase => {
                let first = ident.chars().next().unwrap();
                first.is_ascii_uppercase()
                    && ident.chars().all(|c| c.is_ascii_alphanumeric())
            }
            Self::CamelCase => {
                let first = ident.chars().next().unwrap();
                first.is_ascii_lowercase()
                    && ident.chars().all(|c| c.is_ascii_alphanumeric())
            }
            Self::ScreamingSnakeCase => ident
                .chars()
                .all(|c| c == '_' || c.is_ascii_uppercase() || c.is_ascii_digit()),
            Self::Lowercase => ident.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            Self::Uppercase => ident.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()),
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::SnakeCase => "snake_case",
            Self::KebabCase => "kebab-case",
            Self::PascalCase => "PascalCase",
            Self::CamelCase => "camelCase",
            Self::ScreamingSnakeCase => "SCREAMING_SNAKE_CASE",
            Self::Lowercase => "lowercase",
            Self::Uppercase => "UPPERCASE",
        }
    }
}

struct NamingConventionPass;

impl LintPass for NamingConventionPass {
    fn name(&self) -> &'static str { "naming-convention" }
    fn description(&self) -> &'static str {
        "Identifier doesn't match the project's [lint.naming] convention"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Warning }
    fn category(&self) -> LintCategory { LintCategory::Style }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        // Read config; fall back to defaults if absent.
        let cfg: NamingConfig = ctx
            .config
            .and_then(|c| c.rule_config::<NamingConfig>("naming-convention"))
            .unwrap_or_default();

        let fn_casing = Casing::parse(&cfg.fn_case).unwrap_or(Casing::SnakeCase);
        let type_casing = Casing::parse(&cfg.type_case).unwrap_or(Casing::PascalCase);
        let const_casing = Casing::parse(&cfg.const_case).unwrap_or(Casing::ScreamingSnakeCase);

        let mut issues = Vec::new();

        for item in &ctx.module.items {
            match &item.kind {
                ItemKind::Function(f) => {
                    let name = f.name.as_str();
                    if cfg.exempt.fn_names.iter().any(|x| x == name) {
                        continue;
                    }
                    if !fn_casing.matches(name) {
                        let (line, col) = span_to_line_col(ctx.source, item.span.start);
                        issues.push(LintIssue {
                            rule: "naming-convention",
                            level: LintLevel::Warning,
                            file: ctx.file.to_path_buf(),
                            line,
                            column: col,
                            message: format!(
                                "fn `{}` doesn't match {} convention",
                                name, fn_casing.description()
                            ),
                            suggestion: None,
                            fixable: false,
                        });
                    }
                }
                ItemKind::Type(t) => {
                    let name = t.name.as_str();
                    if cfg.exempt.type_names.iter().any(|x| x == name) {
                        continue;
                    }
                    if !type_casing.matches(name) {
                        let (line, col) = span_to_line_col(ctx.source, item.span.start);
                        issues.push(LintIssue {
                            rule: "naming-convention",
                            level: LintLevel::Warning,
                            file: ctx.file.to_path_buf(),
                            line,
                            column: col,
                            message: format!(
                                "type `{}` doesn't match {} convention",
                                name, type_casing.description()
                            ),
                            suggestion: None,
                            fixable: false,
                        });
                    }
                }
                ItemKind::Const(c) => {
                    let name = c.name.as_str();
                    if cfg.exempt.const_names.iter().any(|x| x == name) {
                        continue;
                    }
                    if !const_casing.matches(name) {
                        let (line, col) = span_to_line_col(ctx.source, item.span.start);
                        issues.push(LintIssue {
                            rule: "naming-convention",
                            level: LintLevel::Warning,
                            file: ctx.file.to_path_buf(),
                            line,
                            column: col,
                            message: format!(
                                "const `{}` doesn't match {} convention",
                                name, const_casing.description()
                            ),
                            suggestion: None,
                            fixable: false,
                        });
                    }
                }
                _ => {}
            }
        }
        issues
    }
}

// ===================================================================
// In-source suppression: @allow / @deny / @warn(rule, reason = "...")
// ===================================================================
//
// Verum-idiomatic call-site control over lint severity. Three
// attribute names — `@allow`, `@deny`, `@warn` — accept a string-
// literal rule name plus an optional `reason = "..."` named arg.
//
//     @allow("unused-import", reason = "re-export for derive")
//     @deny("todo-in-code")
//     @warn("deprecated-syntax")
//
// Why string literals: kebab-case rule names (`unused-import`,
// `cbgr-hotspot`) cannot parse as Verum identifiers — `unused-import`
// would parse as `unused - import` (subtraction). Strings are also
// what `[lint.severity]` keys look like, keeping the in-source
// surface and the manifest surface in lockstep.
//
// ===================================================================

/// What an in-source attribute does to the severity of a rule
/// inside its enclosing item's span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressAction {
    /// Drop the diagnostic entirely (`@allow`).
    Allow,
    /// Promote to error (`@deny`).
    Deny,
    /// Force severity warn (`@warn`).
    Warn,
}

/// A single suppression scope — extracted from one attribute, scoped
/// to one item's source span.
#[derive(Debug, Clone)]
pub struct SuppressionScope {
    pub rule: String,
    pub action: SuppressAction,
    pub reason: Option<String>,
    /// 1-based inclusive line range covered by this suppression.
    pub line_start: usize,
    pub line_end: usize,
}

/// Walk every item in `module` and collect every `@allow / @deny /
/// @warn` attribute on it, scoped to that item's source span. Module-
/// level attributes (in `module.attributes`) cover the whole file.
pub fn collect_suppressions(module: &Module, source: &str) -> Vec<SuppressionScope> {
    let mut out = Vec::new();

    // Module-level attributes apply to every line of the file.
    let last_line = source.lines().count().max(1);
    extract_from_attrs(&module.attributes, 1, last_line, &mut out);

    // Item-level attributes apply to the item's span.
    for item in &module.items {
        let attrs = item_attributes(item);
        if attrs.is_empty() {
            continue;
        }
        let (start_line, _) = span_to_line_col(source, item.span.start);
        let (mut end_line, _) = span_to_line_col(source, item.span.end);
        if end_line < start_line {
            end_line = start_line;
        }
        extract_from_attrs(attrs, start_line, end_line, &mut out);
    }

    out
}

/// Apply suppressions to a list of issues. Allow → drop, Deny →
/// raise to Error, Warn → demote/promote to Warning.
///
/// An issue is matched to a suppression iff the issue's `line` is
/// inside the suppression's [line_start, line_end] inclusive range
/// AND the suppression's `rule` matches the issue's rule name.
///
/// Multiple matching suppressions: most-specific (smallest line span)
/// wins.
pub fn apply_suppressions(
    mut issues: Vec<LintIssue>,
    scopes: &[SuppressionScope],
) -> Vec<LintIssue> {
    issues.retain_mut(|issue| {
        // Pick the smallest-spanning matching suppression.
        let mut best: Option<&SuppressionScope> = None;
        for s in scopes {
            if s.rule != issue.rule {
                continue;
            }
            if issue.line < s.line_start || issue.line > s.line_end {
                continue;
            }
            let s_size = s.line_end - s.line_start;
            best = match best {
                None => Some(s),
                Some(prev) if prev.line_end - prev.line_start > s_size => Some(s),
                Some(prev) => Some(prev),
            };
        }
        if let Some(s) = best {
            match s.action {
                SuppressAction::Allow => return false, // drop the issue
                SuppressAction::Deny => issue.level = LintLevel::Error,
                SuppressAction::Warn => issue.level = LintLevel::Warning,
            }
        }
        true
    });
    issues
}

fn extract_from_attrs(
    attrs: &[Attribute],
    line_start: usize,
    line_end: usize,
    out: &mut Vec<SuppressionScope>,
) {
    use verum_ast::{ExprKind, LiteralKind};
    for a in attrs {
        let action = match a.name.as_str() {
            "allow" => SuppressAction::Allow,
            "deny" => SuppressAction::Deny,
            "warn" => SuppressAction::Warn,
            _ => continue,
        };
        let args = match &a.args {
            verum_common::Maybe::Some(args) => args,
            _ => continue,
        };
        let mut rule_name: Option<String> = None;
        let mut reason: Option<String> = None;
        for e in args.iter() {
            match &e.kind {
                ExprKind::Literal(lit) => {
                    if let LiteralKind::Text(s) = &lit.kind {
                        // StringLit::Display wraps in quotes; we need
                        // the unquoted content so the rule name
                        // matches `[lint.severity]` keys exactly.
                        if rule_name.is_none() {
                            rule_name = Some(s.as_str().to_string());
                        }
                    }
                }
                ExprKind::NamedArg { name, value } => {
                    if name.as_str() == "reason" {
                        if let ExprKind::Literal(lit) = &value.kind {
                            if let LiteralKind::Text(s) = &lit.kind {
                                reason = Some(s.as_str().to_string());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(rule) = rule_name {
            out.push(SuppressionScope {
                rule,
                action,
                reason,
                line_start,
                line_end,
            });
        }
    }
}

fn item_attributes(item: &verum_ast::Item) -> &[Attribute] {
    use verum_ast::ItemKind::*;
    match &item.kind {
        Function(f) => f.attributes.as_slice(),
        Type(t) => t.attributes.as_slice(),
        Theorem(t) | Lemma(t) | Corollary(t) => t.attributes.as_slice(),
        Axiom(a) => a.attributes.as_slice(),
        // Other item kinds (Mount, Const, Static, Protocol, Module,
        // Pattern, ExternBlock, …) don't carry attribute lists in the
        // current AST. `@allow`/`@deny`/`@warn` placed on them is
        // silently ignored — same as on a comment. Add support here
        // when the corresponding decl gains an `attributes` field.
        _ => &[],
    }
}

// ===================================================================
// Phase C.1: Refinement-policy enforcement
// ===================================================================
//
// Three passes that police how a project uses Verum's refinement-type
// system. Configured via the synthetic rule key
// `refinement-policy` populated from the `[lint.refinement_policy]`
// manifest block:
//
//     [lint.refinement_policy]
//     public_api_must_refine_int      = true
//     require_verify_on_refined_fn    = true
//     disallow_redundant_refinements  = true
//
// Each policy is a separate rule so users can dial them independently
// via `[lint.severity]` or `@allow / @deny / @warn`.
//
// ===================================================================

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
struct RefinementPolicyConfig {
    public_api_must_refine_int: bool,
    public_api_must_refine_text: bool,
    require_verify_on_refined_fn: bool,
}

impl Default for RefinementPolicyConfig {
    fn default() -> Self {
        Self {
            public_api_must_refine_int: false,
            public_api_must_refine_text: false,
            require_verify_on_refined_fn: false,
        }
    }
}

fn refinement_policy(ctx: &LintCtx<'_>) -> RefinementPolicyConfig {
    ctx.config
        .and_then(|c| c.rule_config::<RefinementPolicyConfig>("refinement-policy"))
        .unwrap_or_default()
}

/// Recurse into `Refined { base, ... }` chains to reveal the
/// underlying base type (Int, Text, …). Returns the base reference.
fn unwrap_refinement_kind(ty: &Type) -> &TypeKind {
    match &ty.kind {
        TypeKind::Refined { base, .. } => unwrap_refinement_kind(base),
        _ => &ty.kind,
    }
}

/// True iff `ty` is a refinement (one or more `Refined { base, .. }`
/// layers wrapping the actual base).
fn is_refined(ty: &Type) -> bool {
    matches!(ty.kind, TypeKind::Refined { .. })
}

/// True iff the function is publicly visible — public, public(crate),
/// public(super), or path-restricted public. Internal / private fns
/// don't trigger public-API policies.
fn is_public_fn(func: &FunctionDecl) -> bool {
    use verum_ast::Visibility::*;
    matches!(
        func.visibility,
        Public | PublicCrate | PublicSuper | PublicIn(_)
    )
}

// ── unrefined-public-int ────────────────────────────────────────────
// Public function takes (or returns) an `Int` / `Text` parameter
// without a refinement. The type system has no way to express a
// usage constraint — every caller can pass any value, and any bug is
// only caught at runtime.
//
// Fires when `[lint.refinement_policy].public_api_must_refine_int`
// (or `.public_api_must_refine_text`) is true. Off by default;
// projects opt in by flipping the flag.

struct UnrefinedPublicIntPass;

impl LintPass for UnrefinedPublicIntPass {
    fn name(&self) -> &'static str { "unrefined-public-int" }
    fn description(&self) -> &'static str {
        "Public fn parameter or return is Int/Text without a refinement — \
         tighten the type to express valid usage at the type level"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Warning }
    fn category(&self) -> LintCategory { LintCategory::Verification }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        let policy = refinement_policy(ctx);
        if !policy.public_api_must_refine_int && !policy.public_api_must_refine_text {
            return Vec::new();
        }
        let mut issues = Vec::new();
        for item in &ctx.module.items {
            let func = match &item.kind {
                ItemKind::Function(f) => f,
                _ => continue,
            };
            if !is_public_fn(func) {
                continue;
            }
            // Walk parameters
            for param in &func.params {
                if let verum_ast::FunctionParamKind::Regular { ty, .. } = &param.kind {
                    if let Some(reason) = check_unrefined_int_or_text(ty, &policy) {
                        let (line, col) = span_to_line_col(ctx.source, param.span.start);
                        issues.push(LintIssue {
                            rule: "unrefined-public-int",
                            level: LintLevel::Warning,
                            file: ctx.file.to_path_buf(),
                            line,
                            column: col,
                            message: format!(
                                "public fn `{}` takes an unrefined {} parameter — \
                                 add a refinement like `{}{{ … }}`",
                                func.name, reason, reason
                            ),
                            suggestion: None,
                            fixable: false,
                        });
                    }
                }
            }
            // Walk return type
            if let Some(ret) = func.return_type.as_ref() {
                if let Some(reason) = check_unrefined_int_or_text(ret, &policy) {
                    let (line, col) = span_to_line_col(ctx.source, ret.span.start);
                    issues.push(LintIssue {
                        rule: "unrefined-public-int",
                        level: LintLevel::Warning,
                        file: ctx.file.to_path_buf(),
                        line,
                        column: col,
                        message: format!(
                            "public fn `{}` returns unrefined {} — \
                             add a refinement to express the postcondition",
                            func.name, reason
                        ),
                        suggestion: None,
                        fixable: false,
                    });
                }
            }
        }
        issues
    }
}

fn check_unrefined_int_or_text(ty: &Type, policy: &RefinementPolicyConfig) -> Option<&'static str> {
    if is_refined(ty) {
        return None;
    }
    match unwrap_refinement_kind(ty) {
        TypeKind::Int if policy.public_api_must_refine_int => Some("Int"),
        TypeKind::Text if policy.public_api_must_refine_text => Some("Text"),
        _ => None,
    }
}

// ── verify-implied-by-refinement ────────────────────────────────────
// A function that uses refinement types in its parameters or return
// MUST carry a `@verify(...)` annotation, otherwise the obligation
// expressed by the refinement is checked only at runtime — losing
// the static-verification value of refinement types.

struct VerifyImpliedByRefinementPass;

impl LintPass for VerifyImpliedByRefinementPass {
    fn name(&self) -> &'static str { "verify-implied-by-refinement" }
    fn description(&self) -> &'static str {
        "Function uses refinement types but lacks @verify — \
         the type-level obligation will only be checked at runtime"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Warning }
    fn category(&self) -> LintCategory { LintCategory::Verification }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        let policy = refinement_policy(ctx);
        if !policy.require_verify_on_refined_fn {
            return Vec::new();
        }
        let mut issues = Vec::new();
        for item in &ctx.module.items {
            let func = match &item.kind {
                ItemKind::Function(f) => f,
                _ => continue,
            };
            // Has @verify already? Done.
            if attrs_contain(&func.attributes, "verify") {
                continue;
            }
            let has_refined_param = func.params.iter().any(|p| {
                if let verum_ast::FunctionParamKind::Regular { ty, .. } = &p.kind {
                    is_refined(ty)
                } else {
                    false
                }
            });
            let has_refined_return =
                func.return_type.as_ref().map(|t| is_refined(t)).unwrap_or(false);
            if has_refined_param || has_refined_return {
                let (line, col) = span_to_line_col(ctx.source, item.span.start);
                issues.push(LintIssue {
                    rule: "verify-implied-by-refinement",
                    level: LintLevel::Warning,
                    file: ctx.file.to_path_buf(),
                    line,
                    column: col,
                    message: format!(
                        "fn `{}` uses refinement types but lacks @verify(...) — \
                         add @verify(formal) so the obligation is statically checked",
                        func.name
                    ),
                    suggestion: None,
                    fixable: false,
                });
            }
        }
        issues
    }
}

// ── public-must-have-verify ─────────────────────────────────────────
// Configured via `[lint.verification_policy].public_must_have_verify`.
// Every public function should carry a `@verify(...)` attribute —
// from `runtime` (no proof, just runtime asserts) to `formal` (full
// SMT proof). The default is "off" because not every project wants
// every public fn formally verified, but for security-critical
// codebases this is the policy that turns "you forgot @verify" into
// a build error.

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
struct VerificationPolicyConfig {
    public_must_have_verify: bool,
}

impl Default for VerificationPolicyConfig {
    fn default() -> Self {
        Self { public_must_have_verify: false }
    }
}

struct PublicMustHaveVerifyPass;

impl LintPass for PublicMustHaveVerifyPass {
    fn name(&self) -> &'static str { "public-must-have-verify" }
    fn description(&self) -> &'static str {
        "Public function lacks @verify(...) — declare its verification \
         strategy explicitly (runtime | static | formal | …)"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Hint }
    fn category(&self) -> LintCategory { LintCategory::Verification }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        let policy: VerificationPolicyConfig = ctx
            .config
            .and_then(|c| c.rule_config::<VerificationPolicyConfig>("verification-policy"))
            .unwrap_or_default();
        if !policy.public_must_have_verify {
            return Vec::new();
        }
        let mut issues = Vec::new();
        for item in &ctx.module.items {
            let func = match &item.kind {
                ItemKind::Function(f) => f,
                _ => continue,
            };
            if !is_public_fn(func) {
                continue;
            }
            if attrs_contain(&func.attributes, "verify") {
                continue;
            }
            let (line, col) = span_to_line_col(ctx.source, item.span.start);
            issues.push(LintIssue {
                rule: "public-must-have-verify",
                level: LintLevel::Hint,
                file: ctx.file.to_path_buf(),
                line,
                column: col,
                message: format!(
                    "public fn `{}` lacks @verify(...) — declare its \
                     verification strategy explicitly",
                    func.name
                ),
                suggestion: None,
                fixable: false,
            });
        }
        issues
    }
}

// ===================================================================
// Phase C.3: Context-policy enforcement (`using [...]`)
// ===================================================================
//
// Per-module `using [...]` policy. Configured via the synthetic rule
// key `context-policy` populated from `[lint.context_policy.modules]`:
//
//     [lint.context_policy.modules]
//     "core.*"        = { forbid    = ["Database", "Logger", "Clock"] }
//     "core.math.*"   = { forbid_all = true }
//     "app.handlers"  = { allow     = ["Database", "Logger"] }
//
// The pass walks every function's `using [...]` requirement list,
// resolves the most-specific applicable rule for the current file's
// module path, and fires when:
//
//   - `forbid_all = true` and the function uses any context, OR
//   - `forbid = […]` contains the requested context, OR
//   - `allow  = […]` is set and the requested context is NOT in it.
//
// "Most-specific applicable rule": longer pattern wins
// ("core.math.*" beats "core.*"). Exact path match beats any glob.
//
// ===================================================================

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
struct ContextPolicyConfig {
    /// modules.<glob> = { forbid|allow|forbid_all }
    modules: std::collections::HashMap<String, ContextRule>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
struct ContextRule {
    forbid: Vec<String>,
    allow: Vec<String>,
    forbid_all: bool,
}

/// Resolve the module path for a file. `src/` is treated as a build
/// directory (stripped); `core/`, `tests/`, `benches/`, `app/` etc.
/// are legitimate top-level namespaces (kept). Translates `/` to `.`,
/// strips the `.vr` extension. Robust against absolute paths — finds
/// the *last* `src/` segment when present.
///
/// Examples:
///   `core/math/linalg.vr`       → `core.math.linalg`
///   `src/foo/bar.vr`            → `foo.bar`
///   `/abs/path/to/src/lib.vr`   → `lib`
///   `tests/integration.vr`      → `tests.integration`
fn module_path_for_file(file: &Path) -> String {
    let s = file.to_string_lossy();
    // Strip the build directory if it appears in the path — but only
    // `src/`. Other top-level dirs (core/tests/benches/app/…) are
    // legitimate module namespaces and stay.
    let trimmed = if let Some(idx) = s.rfind("/src/") {
        &s[idx + 5..]
    } else if let Some(rest) = s.strip_prefix("src/") {
        rest
    } else {
        s.as_ref()
    };
    let stem = trimmed.strip_suffix(".vr").unwrap_or(trimmed);
    stem.replace('/', ".")
}

/// Glob match for module paths. Pattern semantics:
///
///   - `*` at the end of a segment matches that whole segment.
///   - `*` as a sole segment matches any one segment.
///   - `**` matches zero or more segments (greedy).
///   - Exact match otherwise.
///
/// Examples:
///   pattern "core"          ↔ "core"                     ✓ exact
///   pattern "core.*"        ↔ "core.foo", "core.foo.bar" ✓
///   pattern "core.math.*"   ↔ "core.math.linalg"         ✓
///   pattern "core.math.*"   ↔ "core.parser"              ✗
fn glob_module_match(pattern: &str, path: &str) -> bool {
    if pattern == path {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return path.starts_with(prefix)
            && (path.len() == prefix.len() || path.as_bytes().get(prefix.len()) == Some(&b'.'));
    }
    if pattern == "*" {
        return !path.is_empty();
    }
    if let Some(prefix) = pattern.strip_suffix(".**") {
        return path == prefix || path.starts_with(&format!("{}.", prefix));
    }
    false
}

/// Pick the most-specific matching rule for `module_path`. Pattern
/// length is the tiebreak — longer patterns win because they encode
/// more constraints than shorter ones.
fn resolve_context_rule<'a>(
    cfg: &'a ContextPolicyConfig,
    module_path: &str,
) -> Option<(&'a String, &'a ContextRule)> {
    cfg.modules
        .iter()
        .filter(|(pat, _)| glob_module_match(pat, module_path))
        .max_by_key(|(pat, _)| pat.len())
}

struct ForbiddenContextPass;

impl LintPass for ForbiddenContextPass {
    fn name(&self) -> &'static str { "forbidden-context" }
    fn description(&self) -> &'static str {
        "Function uses a context (`using [X]`) that the project's \
         [lint.context_policy.modules] forbids in this module path"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Error }
    fn category(&self) -> LintCategory { LintCategory::Safety }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        let cfg: ContextPolicyConfig = match ctx
            .config
            .and_then(|c| c.rule_config::<ContextPolicyConfig>("context-policy"))
        {
            Some(c) if !c.modules.is_empty() => c,
            _ => return Vec::new(),
        };

        let module_path = module_path_for_file(ctx.file);
        let (matched_pattern, rule) = match resolve_context_rule(&cfg, &module_path) {
            Some(x) => x,
            None => return Vec::new(),
        };

        let mut issues = Vec::new();
        for item in &ctx.module.items {
            let func = match &item.kind {
                ItemKind::Function(f) => f,
                _ => continue,
            };
            for req in &func.contexts {
                let name = path_segments_to_string(&req.path);
                let violated = if rule.forbid_all {
                    Some("forbid_all = true")
                } else if rule.forbid.iter().any(|n| n == &name) {
                    Some("listed in `forbid`")
                } else if !rule.allow.is_empty() && !rule.allow.iter().any(|n| n == &name) {
                    Some("not listed in `allow`")
                } else {
                    None
                };
                if let Some(why) = violated {
                    let (line, col) = span_to_line_col(ctx.source, req.span.start);
                    issues.push(LintIssue {
                        rule: "forbidden-context",
                        level: LintLevel::Error,
                        file: ctx.file.to_path_buf(),
                        line,
                        column: col,
                        message: format!(
                            "module `{}` may not use context `{}` (matched pattern `{}`, {})",
                            module_path, name, matched_pattern, why
                        ),
                        suggestion: None,
                        fixable: false,
                    });
                }
            }
        }
        issues
    }
}

/// Convert a Verum `Path` (sequence of segments) to a dotted string.
/// Used when comparing context names to allow/forbid lists.
fn path_segments_to_string(p: &verum_ast::Path) -> String {
    let mut out = String::new();
    for (i, seg) in p.segments.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        match seg {
            verum_ast::PathSegment::Name(id) => out.push_str(id.name.as_str()),
            verum_ast::PathSegment::SelfValue => out.push_str("self"),
            verum_ast::PathSegment::Super => out.push_str("super"),
            verum_ast::PathSegment::Cog => out.push_str("cog"),
            verum_ast::PathSegment::Relative => out.push('.'),
        }
    }
    out
}

// ===================================================================
// Phase B.4: Architecture-layering enforcement
// ===================================================================
//
// Module-import constraints turned into compile-time errors. Configured
// via `[lint.architecture.layers]` (allow-list per layer) and
// `[lint.architecture.bans]` (explicit deny pairs):
//
//   [lint.architecture.layers]
//   core    = { allow_imports = ["core", "std"] }
//   domain  = { allow_imports = ["core", "std", "domain"] }
//   adapter = { allow_imports = ["core", "std", "domain", "adapter"] }
//
//   [lint.architecture.bans]
//   "app.ui"      = ["app.persistence", "app.network"]
//   "core.crypto" = ["core.testing"]
//
// Resolution: each file's module path picks the most-specific layer
// by prefix match. Every `mount X.Y.Z` in that file is then checked
// against the layer's `allow_imports` (whitelist) plus any explicit
// ban entry. Both checks must pass; either can fire the rule.
//
// ===================================================================

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
struct ArchitectureConfig {
    layers: std::collections::HashMap<String, LayerRule>,
    bans: std::collections::HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
struct LayerRule {
    allow_imports: Vec<String>,
}

/// Pick the layer whose name is the longest prefix of `module_path`.
/// Layer names are top-segment prefixes (`"core"`, `"domain"`, …);
/// match by checking `module_path == name` or `module_path.starts_with(name + ".")`.
fn resolve_layer<'a>(
    cfg: &'a ArchitectureConfig,
    module_path: &str,
) -> Option<(&'a String, &'a LayerRule)> {
    cfg.layers
        .iter()
        .filter(|(name, _)| {
            module_path == name.as_str()
                || module_path
                    .as_bytes()
                    .get(name.len())
                    .map(|b| *b == b'.')
                    .unwrap_or(false)
                    && module_path.starts_with(name.as_str())
        })
        .max_by_key(|(name, _)| name.len())
}

/// True iff `path` is in the layer's allow_imports (matched as a
/// top-segment prefix). `"core"` allows `core`, `core.foo`, etc.
fn layer_allows(rule: &LayerRule, imported_path: &str) -> bool {
    rule.allow_imports.is_empty() // empty allow-list = unconstrained
        || rule.allow_imports.iter().any(|allowed| {
            imported_path == allowed
                || (imported_path.len() > allowed.len()
                    && imported_path.starts_with(allowed)
                    && imported_path.as_bytes()[allowed.len()] == b'.')
        })
}

/// Convert a `MountDecl` to the dotted path being imported. Returns
/// the ROOT path of the mount — for `mount foo.bar.{Baz, Qux}` that's
/// `foo.bar`; for `mount foo.bar.*` it's `foo.bar`. Unrepresentable
/// shapes (relative paths, super, …) return None.
fn mount_root_path(mount: &verum_ast::MountDecl) -> Option<String> {
    let path = match &mount.tree.kind {
        verum_ast::MountTreeKind::Path(p) => p,
        verum_ast::MountTreeKind::Glob(p) => p,
        verum_ast::MountTreeKind::Nested { prefix, .. } => prefix,
    };
    Some(path_segments_to_string(path))
}

struct ArchitectureViolationPass;

impl LintPass for ArchitectureViolationPass {
    fn name(&self) -> &'static str { "architecture-violation" }
    fn description(&self) -> &'static str {
        "`mount` crosses a layer boundary or matches an explicit ban — \
         the project's [lint.architecture] forbids this import"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Error }
    fn category(&self) -> LintCategory { LintCategory::Style }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        let cfg: ArchitectureConfig = match ctx
            .config
            .and_then(|c| c.rule_config::<ArchitectureConfig>("architecture-policy"))
        {
            Some(c) if !c.layers.is_empty() || !c.bans.is_empty() => c,
            _ => return Vec::new(),
        };

        let module_path = module_path_for_file(ctx.file);
        let layer_match = resolve_layer(&cfg, &module_path);

        // Build the explicit ban list for this module path. A ban
        // key acts as a top-segment prefix — `"core.crypto"` covers
        // `core.crypto` itself plus everything under it. Most-
        // specific (longest) ban-key wins on overlap.
        let ban_match = cfg
            .bans
            .iter()
            .filter(|(pat, _)| {
                // Exact match
                module_path == pat.as_str()
                    // Or prefix: pat must end at a `.`-boundary in module_path
                    || (module_path.len() > pat.len()
                        && module_path.starts_with(pat.as_str())
                        && module_path.as_bytes()[pat.len()] == b'.')
                    // Or pat is a glob
                    || glob_module_match(pat, &module_path)
            })
            .max_by_key(|(pat, _)| pat.len());

        let mut issues = Vec::new();
        for item in &ctx.module.items {
            let mount = match &item.kind {
                ItemKind::Mount(m) => m,
                _ => continue,
            };
            let imported_path = match mount_root_path(mount) {
                Some(p) if !p.is_empty() => p,
                _ => continue,
            };

            // Check layer allow-list.
            if let Some((layer_name, rule)) = layer_match {
                if !layer_allows(rule, &imported_path) {
                    let (line, col) = span_to_line_col(ctx.source, mount.span.start);
                    issues.push(LintIssue {
                        rule: "architecture-violation",
                        level: LintLevel::Error,
                        file: ctx.file.to_path_buf(),
                        line,
                        column: col,
                        message: format!(
                            "module `{}` (layer `{}`) may not import `{}` — \
                             not in `allow_imports`",
                            module_path, layer_name, imported_path
                        ),
                        suggestion: None,
                        fixable: false,
                    });
                    continue; // already reported; don't double-fire on bans
                }
            }

            // Check explicit bans.
            if let Some((ban_key, banned)) = ban_match {
                if banned.iter().any(|b| {
                    imported_path == *b
                        || (imported_path.len() > b.len()
                            && imported_path.starts_with(b)
                            && imported_path.as_bytes()[b.len()] == b'.')
                }) {
                    let (line, col) = span_to_line_col(ctx.source, mount.span.start);
                    issues.push(LintIssue {
                        rule: "architecture-violation",
                        level: LintLevel::Error,
                        file: ctx.file.to_path_buf(),
                        line,
                        column: col,
                        message: format!(
                            "module `{}` may not import `{}` — \
                             explicit ban (matched `{}`)",
                            module_path, imported_path, ban_key
                        ),
                        suggestion: None,
                        fixable: false,
                    });
                }
            }
        }
        issues
    }
}

// ===================================================================
// Phase C.4: CBGR-budget enforcement
// ===================================================================
//
// Per-module `max_check_ns` budget for managed CBGR references.
// Configured via `[lint.cbgr_budgets]`:
//
//     [lint.cbgr_budgets]
//     default_check_ns = 15
//
//     [lint.cbgr_budgets.modules]
//     "app.handlers.*" = { max_check_ns = 30 }
//     "core.runtime.*" = { max_check_ns = 0  }    # 0 = managed refs forbidden
//
// The pass walks every `UnOp::Ref` / `UnOp::RefMut` (Tier-0, ~15ns
// per deref) in the module's expressions and compares against the
// budget for the current module path. Today's enforcement is static:
//
//   max_check_ns = 0      → every managed ref is reported
//   max_check_ns < 15     → every managed ref is reported (budget
//                            < the cheapest single check)
//   max_check_ns >= 15    → silent (within static estimate)
//
// Profile-driven enforcement (compare measured runtime cost from
// `target/profile/last.json` against the budget) is Stage 3.
//
// ===================================================================

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
struct CbgrBudgetsConfig {
    default_check_ns: u64,
    modules: std::collections::HashMap<String, CbgrModuleBudget>,
}

impl Default for CbgrBudgetsConfig {
    fn default() -> Self {
        Self {
            default_check_ns: 15, // matches CBGR spec: ~15ns per managed deref
            modules: std::collections::HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
struct CbgrModuleBudget {
    max_check_ns: u64,
}

/// Cost of one managed-tier CBGR reference deref, per the CBGR spec
/// (`docs/detailed/26-cbgr-implementation.md`). This is the static
/// estimate; profile-driven enforcement is Stage 3.
const CBGR_MANAGED_DEREF_NS: u64 = 15;

struct CbgrBudgetExceededPass;

impl LintPass for CbgrBudgetExceededPass {
    fn name(&self) -> &'static str { "cbgr-budget-exceeded" }
    fn description(&self) -> &'static str {
        "Managed CBGR reference (`&` / `&mut`) used in a module whose \
         [lint.cbgr_budgets].max_check_ns budget is below the static \
         per-deref cost — promote to `&checked` (0ns) or `&unsafe`"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Warning }
    fn category(&self) -> LintCategory { LintCategory::Performance }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        let cfg: CbgrBudgetsConfig = match ctx
            .config
            .and_then(|c| c.rule_config::<CbgrBudgetsConfig>("cbgr-budgets"))
        {
            Some(c) => c,
            None => return Vec::new(),
        };

        let module_path = module_path_for_file(ctx.file);
        // Most-specific module budget; fall back to default_check_ns.
        let budget = cfg
            .modules
            .iter()
            .filter(|(pat, _)| {
                module_path == pat.as_str()
                    || (module_path.len() > pat.len()
                        && module_path.starts_with(pat.as_str())
                        && module_path.as_bytes()[pat.len()] == b'.')
                    || glob_module_match(pat, &module_path)
            })
            .max_by_key(|(pat, _)| pat.len())
            .map(|(_, b)| b.max_check_ns)
            .unwrap_or(cfg.default_check_ns);

        // Static estimate: every managed deref costs at least
        // CBGR_MANAGED_DEREF_NS. If the budget is below that, every
        // managed ref violates the budget.
        if budget >= CBGR_MANAGED_DEREF_NS {
            return Vec::new();
        }

        // Walk every Ref / RefMut occurrence in fn bodies.
        struct V<'s, 'p> {
            source: &'s str,
            file: &'p Path,
            budget: u64,
            module_path: String,
            issues: Vec<LintIssue>,
        }
        impl<'s, 'p> Visitor for V<'s, 'p> {
            fn visit_expr(&mut self, expr: &verum_ast::Expr) {
                use verum_ast::{ExprKind, UnOp};
                if let ExprKind::Unary { op, .. } = &expr.kind {
                    if matches!(op, UnOp::Ref | UnOp::RefMut) {
                        let (line, col) = span_to_line_col(self.source, expr.span.start);
                        let promote_hint = if matches!(op, UnOp::Ref) {
                            "&checked"
                        } else {
                            "&checked mut"
                        };
                        self.issues.push(LintIssue {
                            rule: "cbgr-budget-exceeded",
                            level: LintLevel::Warning,
                            file: self.file.to_path_buf(),
                            line,
                            column: col,
                            message: format!(
                                "managed `{}` reference in `{}` exceeds budget \
                                 ({}ns budget < {}ns/deref); promote to `{}` for 0ns",
                                op.as_str(),
                                self.module_path,
                                self.budget,
                                CBGR_MANAGED_DEREF_NS,
                                promote_hint,
                            ),
                            suggestion: None,
                            fixable: false,
                        });
                    }
                }
                visitor::walk_expr(self, expr);
            }
        }
        let mut v = V {
            source: ctx.source,
            file: ctx.file,
            budget,
            module_path,
            issues: Vec::new(),
        };
        for item in &ctx.module.items {
            v.visit_item(item);
        }
        v.issues
    }
}

// ===================================================================
// Phase C.6: Style ceilings + Phase C.5: documentation policy
// ===================================================================
//
// Configured via `[lint.style]` and `[lint.documentation]`:
//
//     [lint.style]
//     max_line_length          = 100
//     max_fn_lines             = 80
//     max_fn_params            = 5
//     max_match_arms           = 12
//
//     [lint.documentation]
//     public_must_have_doc     = true
//
// Each ceiling is its own LintPass so users can dial them
// individually via `[lint.severity]` / `@allow / @deny / @warn`.
//
// ===================================================================

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
struct StylePolicyConfig {
    max_line_length: u32,
    max_fn_lines: u32,
    max_fn_params: u32,
    max_match_arms: u32,
}

impl Default for StylePolicyConfig {
    fn default() -> Self {
        Self {
            max_line_length: 100,
            max_fn_lines: 80,
            max_fn_params: 5,
            max_match_arms: 12,
        }
    }
}

fn style_policy(ctx: &LintCtx<'_>) -> Option<StylePolicyConfig> {
    ctx.config
        .and_then(|c| c.rule_config::<StylePolicyConfig>("style-policy"))
}

// ── max-line-length ────────────────────────────────────────────────

struct MaxLineLengthPass;

impl LintPass for MaxLineLengthPass {
    fn name(&self) -> &'static str { "max-line-length" }
    fn description(&self) -> &'static str {
        "Source line exceeds [lint.style].max_line_length characters"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Hint }
    fn category(&self) -> LintCategory { LintCategory::Style }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        let cfg = match style_policy(ctx) {
            Some(c) => c,
            None => return Vec::new(),
        };
        if cfg.max_line_length == 0 {
            return Vec::new();
        }
        let mut issues = Vec::new();
        for (i, line) in ctx.source.lines().enumerate() {
            // Count characters not bytes — UTF-8 width matters.
            let len = line.chars().count() as u32;
            if len > cfg.max_line_length {
                issues.push(LintIssue {
                    rule: "max-line-length",
                    level: LintLevel::Hint,
                    file: ctx.file.to_path_buf(),
                    line: i + 1,
                    column: cfg.max_line_length as usize + 1,
                    message: format!(
                        "line is {} characters; budget is {}",
                        len, cfg.max_line_length
                    ),
                    suggestion: None,
                    fixable: false,
                });
            }
        }
        issues
    }
}

// ── max-fn-lines ────────────────────────────────────────────────────

struct MaxFnLinesPass;

impl LintPass for MaxFnLinesPass {
    fn name(&self) -> &'static str { "max-fn-lines" }
    fn description(&self) -> &'static str {
        "Function body exceeds [lint.style].max_fn_lines"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Hint }
    fn category(&self) -> LintCategory { LintCategory::Style }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        let cfg = match style_policy(ctx) {
            Some(c) => c,
            None => return Vec::new(),
        };
        if cfg.max_fn_lines == 0 {
            return Vec::new();
        }
        let mut issues = Vec::new();
        for item in &ctx.module.items {
            let func = match &item.kind {
                ItemKind::Function(f) => f,
                _ => continue,
            };
            let (start_line, _) = span_to_line_col(ctx.source, item.span.start);
            let (end_line, _) = span_to_line_col(ctx.source, item.span.end);
            let lines = end_line.saturating_sub(start_line).saturating_add(1) as u32;
            if lines > cfg.max_fn_lines {
                issues.push(LintIssue {
                    rule: "max-fn-lines",
                    level: LintLevel::Hint,
                    file: ctx.file.to_path_buf(),
                    line: start_line,
                    column: 1,
                    message: format!(
                        "fn `{}` is {} lines; budget is {}",
                        func.name, lines, cfg.max_fn_lines
                    ),
                    suggestion: None,
                    fixable: false,
                });
            }
        }
        issues
    }
}

// ── max-fn-params ──────────────────────────────────────────────────

struct MaxFnParamsPass;

impl LintPass for MaxFnParamsPass {
    fn name(&self) -> &'static str { "max-fn-params" }
    fn description(&self) -> &'static str {
        "Function takes more parameters than [lint.style].max_fn_params"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Hint }
    fn category(&self) -> LintCategory { LintCategory::Style }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        let cfg = match style_policy(ctx) {
            Some(c) => c,
            None => return Vec::new(),
        };
        if cfg.max_fn_params == 0 {
            return Vec::new();
        }
        let mut issues = Vec::new();
        for item in &ctx.module.items {
            let func = match &item.kind {
                ItemKind::Function(f) => f,
                _ => continue,
            };
            let n = func.params.len() as u32;
            if n > cfg.max_fn_params {
                let (line, col) = span_to_line_col(ctx.source, item.span.start);
                issues.push(LintIssue {
                    rule: "max-fn-params",
                    level: LintLevel::Hint,
                    file: ctx.file.to_path_buf(),
                    line,
                    column: col,
                    message: format!(
                        "fn `{}` takes {} parameters; budget is {}",
                        func.name, n, cfg.max_fn_params
                    ),
                    suggestion: None,
                    fixable: false,
                });
            }
        }
        issues
    }
}

// ── max-match-arms ─────────────────────────────────────────────────

struct MaxMatchArmsPass;

impl LintPass for MaxMatchArmsPass {
    fn name(&self) -> &'static str { "max-match-arms" }
    fn description(&self) -> &'static str {
        "match expression has more arms than [lint.style].max_match_arms"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Hint }
    fn category(&self) -> LintCategory { LintCategory::Style }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        let cfg = match style_policy(ctx) {
            Some(c) => c,
            None => return Vec::new(),
        };
        if cfg.max_match_arms == 0 {
            return Vec::new();
        }

        struct V<'s, 'p> {
            source: &'s str,
            file: &'p Path,
            limit: u32,
            issues: Vec<LintIssue>,
        }
        impl<'s, 'p> Visitor for V<'s, 'p> {
            fn visit_expr(&mut self, expr: &verum_ast::Expr) {
                use verum_ast::ExprKind;
                if let ExprKind::Match { arms, .. } = &expr.kind {
                    let n = arms.len() as u32;
                    if n > self.limit {
                        let (line, col) = span_to_line_col(self.source, expr.span.start);
                        self.issues.push(LintIssue {
                            rule: "max-match-arms",
                            level: LintLevel::Hint,
                            file: self.file.to_path_buf(),
                            line,
                            column: col,
                            message: format!(
                                "match expression has {} arms; budget is {}",
                                n, self.limit
                            ),
                            suggestion: None,
                            fixable: false,
                        });
                    }
                }
                visitor::walk_expr(self, expr);
            }
        }
        let mut v = V {
            source: ctx.source,
            file: ctx.file,
            limit: cfg.max_match_arms,
            issues: Vec::new(),
        };
        for item in &ctx.module.items {
            v.visit_item(item);
        }
        v.issues
    }
}

// ── public-must-have-doc ───────────────────────────────────────────
//
// Phase C.5 documentation policy. Configured via
// [lint.documentation].public_must_have_doc. A public fn/type/const
// without a doc comment fires the rule. Detection: scan the source
// lines immediately preceding the item's start line for `///`. We
// don't rely on AST attribute parsing for doc comments because Verum
// stores them in a separate channel.

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
struct DocumentationPolicyConfig {
    public_must_have_doc: bool,
}

struct PublicMustHaveDocPass;

impl LintPass for PublicMustHaveDocPass {
    fn name(&self) -> &'static str { "public-must-have-doc" }
    fn description(&self) -> &'static str {
        "Public item lacks a doc comment (`///`) — \
         add one or set [lint.documentation].public_must_have_doc = false"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Hint }
    fn category(&self) -> LintCategory { LintCategory::Style }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        let cfg: DocumentationPolicyConfig = ctx
            .config
            .and_then(|c| c.rule_config::<DocumentationPolicyConfig>("documentation-policy"))
            .unwrap_or_default();
        if !cfg.public_must_have_doc {
            return Vec::new();
        }
        let lines: Vec<&str> = ctx.source.lines().collect();
        let mut issues = Vec::new();
        for item in &ctx.module.items {
            let (kind_label, name, is_public) = match &item.kind {
                ItemKind::Function(f) => ("fn", f.name.as_str().to_string(), is_public_fn(f)),
                ItemKind::Type(t) => (
                    "type",
                    t.name.as_str().to_string(),
                    matches!(
                        t.visibility,
                        verum_ast::Visibility::Public
                            | verum_ast::Visibility::PublicCrate
                            | verum_ast::Visibility::PublicSuper
                            | verum_ast::Visibility::PublicIn(_)
                    ),
                ),
                _ => continue,
            };
            if !is_public {
                continue;
            }
            let (item_line, _) = span_to_line_col(ctx.source, item.span.start);
            // Scan the lines immediately above for ///. Skip empty
            // lines and other attributes; stop at the first line that
            // is neither blank nor an attribute / doc comment.
            let mut has_doc = false;
            if item_line >= 2 {
                let mut idx = item_line - 2; // 0-indexed prev line
                loop {
                    let t = lines.get(idx).map(|s| s.trim()).unwrap_or("");
                    if t.starts_with("///") {
                        has_doc = true;
                        break;
                    }
                    if t.is_empty() || t.starts_with("@") || t.starts_with("//") {
                        if idx == 0 {
                            break;
                        }
                        idx -= 1;
                        continue;
                    }
                    break;
                }
            }
            if !has_doc {
                issues.push(LintIssue {
                    rule: "public-must-have-doc",
                    level: LintLevel::Hint,
                    file: ctx.file.to_path_buf(),
                    line: item_line,
                    column: 1,
                    message: format!(
                        "public {} `{}` lacks a `///` doc comment",
                        kind_label, name
                    ),
                    suggestion: None,
                    fixable: false,
                });
            }
        }
        issues
    }
}

// ===================================================================
// Phase C.2: Capability-policy enforcement
// ===================================================================
//
// Configured via `[lint.capability_policy]`:
//
//   [lint.capability_policy]
//   require_cap_for_unsafe = true
//   require_cap_for_ffi    = true
//
// `unsafe-without-capability` (warn, safety):
//   Function declares `unsafe { ... }` blocks but doesn't carry a
//   `@cap(...)` attribute documenting WHY this unsafe is permitted.
//   Verum's capability system is designed to make every safety-
//   relaxation explicit at the type level; this rule turns that
//   convention into a static check.
//
// `ffi-without-capability` (warn, safety):
//   Same idea for FFI declarations — extern blocks and FFI-bound
//   functions should declare `@cap(name = "ffi.libfoo", ...)`.
//
// ===================================================================

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
struct CapabilityPolicyConfig {
    require_cap_for_unsafe: bool,
    require_cap_for_ffi: bool,
}

fn capability_policy(ctx: &LintCtx<'_>) -> CapabilityPolicyConfig {
    ctx.config
        .and_then(|c| c.rule_config::<CapabilityPolicyConfig>("capability-policy"))
        .unwrap_or_default()
}

/// Whether `func` contains any `unsafe { ... }` block in its body.
/// Walks the body via verum_ast::Visitor.
fn fn_uses_unsafe_block(func: &FunctionDecl) -> bool {
    use verum_ast::ExprKind;
    struct V {
        found: bool,
    }
    impl Visitor for V {
        fn visit_expr(&mut self, expr: &verum_ast::Expr) {
            if matches!(&expr.kind, ExprKind::Unsafe(_)) {
                self.found = true;
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut v = V { found: false };
    if let Some(body) = &func.body {
        match body {
            verum_ast::FunctionBody::Block(b) => v.visit_block(b),
            verum_ast::FunctionBody::Expr(e) => v.visit_expr(e),
        }
    }
    v.found
}

struct UnsafeWithoutCapabilityPass;

impl LintPass for UnsafeWithoutCapabilityPass {
    fn name(&self) -> &'static str { "unsafe-without-capability" }
    fn description(&self) -> &'static str {
        "Function uses `unsafe { … }` but lacks @cap(...) — declare \
         the capability explicitly so the trust boundary is auditable"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Warning }
    fn category(&self) -> LintCategory { LintCategory::Safety }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        let policy = capability_policy(ctx);
        if !policy.require_cap_for_unsafe {
            return Vec::new();
        }
        let mut issues = Vec::new();
        for item in &ctx.module.items {
            let func = match &item.kind {
                ItemKind::Function(f) => f,
                _ => continue,
            };
            if attrs_contain(&func.attributes, "cap") {
                continue;
            }
            if !fn_uses_unsafe_block(func) {
                continue;
            }
            let (line, col) = span_to_line_col(ctx.source, item.span.start);
            issues.push(LintIssue {
                rule: "unsafe-without-capability",
                level: LintLevel::Warning,
                file: ctx.file.to_path_buf(),
                line,
                column: col,
                message: format!(
                    "fn `{}` uses `unsafe` block but lacks @cap(...) — \
                     declare a capability so the trust boundary is auditable",
                    func.name
                ),
                suggestion: None,
                fixable: false,
            });
        }
        issues
    }
}

struct FfiWithoutCapabilityPass;

impl LintPass for FfiWithoutCapabilityPass {
    fn name(&self) -> &'static str { "ffi-without-capability" }
    fn description(&self) -> &'static str {
        "FFI item (`@ffi` / `@extern`) lacks @cap(...) — declare \
         the foreign-boundary capability explicitly"
    }
    fn default_level(&self) -> LintLevel { LintLevel::Warning }
    fn category(&self) -> LintCategory { LintCategory::Safety }

    fn check(&self, ctx: &LintCtx<'_>) -> Vec<LintIssue> {
        let policy = capability_policy(ctx);
        if !policy.require_cap_for_ffi {
            return Vec::new();
        }
        let mut issues = Vec::new();
        for item in &ctx.module.items {
            let attrs: &[Attribute] = match &item.kind {
                ItemKind::Function(f) => f.attributes.as_slice(),
                _ => continue,
            };
            // `@ffi(...)` / `@extern(...)` mark FFI items.
            let is_ffi = attrs
                .iter()
                .any(|a| matches!(a.name.as_str(), "ffi" | "extern"));
            if !is_ffi {
                continue;
            }
            if attrs.iter().any(|a| a.name.as_str() == "cap") {
                continue;
            }
            let func_name = match &item.kind {
                ItemKind::Function(f) => f.name.as_str().to_string(),
                _ => "?".to_string(),
            };
            let (line, col) = span_to_line_col(ctx.source, item.span.start);
            issues.push(LintIssue {
                rule: "ffi-without-capability",
                level: LintLevel::Warning,
                file: ctx.file.to_path_buf(),
                line,
                column: col,
                message: format!(
                    "FFI fn `{}` lacks @cap(...) — declare the \
                     foreign-boundary capability explicitly",
                    func_name
                ),
                suggestion: None,
                fixable: false,
            });
        }
        issues
    }
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
        let ctx = LintCtx { file: &path, source: src, module: &module, config: None };
        let issues = RedundantRefinementPass.check(&ctx);
        assert_eq!(issues.len(), 1, "expected one issue, got {:?}", issues);
        assert_eq!(issues[0].rule, "redundant-refinement");
    }

    #[test]
    fn well_formed_refinement_silent() {
        let src = "type Pos is Int{ it > 0 };\n";
        let module = parse_module(src);
        let path = std::path::PathBuf::from("test.vr");
        let ctx = LintCtx { file: &path, source: src, module: &module, config: None };
        assert!(RedundantRefinementPass.check(&ctx).is_empty());
    }

    #[test]
    fn empty_bound_fires() {
        let src = "type Empty is Int{ it > 100 && it < 50 };\n";
        let module = parse_module(src);
        let path = std::path::PathBuf::from("test.vr");
        let ctx = LintCtx { file: &path, source: src, module: &module, config: None };
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
