//! Pre-typecheck safety-feature gates.
//!
//! Walks the parsed AST looking for language constructs that are
//! disabled by the current `[safety]` feature set, emitting clean
//! diagnostics before type-checking runs. Keeping these checks in
//! their own walker avoids threading feature flags through the
//! ~52K-line `TypeChecker` and keeps the gate logic auditable.
//!
//! Currently gates:
//!   - `unsafe { … }` expressions when `safety.unsafe_allowed = false`.

use verum_ast::decl::{FunctionBody, ImplItemKind, Item, ItemKind};
use verum_ast::expr::{Block, ConditionKind};
use verum_ast::stmt::StmtKind;
use verum_ast::{Expr, ExprKind, Module};
use verum_common::List;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};

/// Policy for the safety-gate walker.
///
/// Each flag maps 1:1 to a `[safety]` field in `verum.toml`. When a
/// flag is `true`, the corresponding construct is allowed; when
/// `false`, the walker emits a clean diagnostic pointing at the
/// config key and the `-Z` override.
#[derive(Debug, Clone)]
pub struct SafetyPolicy {
    /// `[safety].unsafe_allowed` — gates `unsafe { ... }` expressions
    /// AND `unsafe fn` declarations.
    pub unsafe_allowed: bool,
    /// `[safety].ffi` — gates `@ffi` / `extern "C"` function
    /// declarations. When false, every extern function is rejected.
    pub ffi: bool,
    /// `[safety].ffi_boundary` — "strict" rejects all FFI without
    /// explicit safety annotation; "lenient" only warns.
    pub ffi_boundary: verum_common::Text,
    /// `[safety].capability_required` — when true, functions using
    /// I/O, network, or unsafe must declare `@capability(...)`.
    pub capability_required: bool,
    /// `[safety].mls_level` — "public"/"secret"/"top_secret".
    /// Higher levels restrict operations available.
    pub mls_level: verum_common::Text,
    /// `[safety].forbid_stdlib_extern` — reject `@extern` from stdlib.
    pub forbid_stdlib_extern: bool,
}

impl SafetyPolicy {
    /// All permissive defaults — no gate fires.
    ///
    /// Note: `ffi_boundary` is "lenient" in this constructor (not the
    /// project-default "strict") so the name "permissive" remains
    /// accurate — strict mode emits a warning on every extern fn
    /// missing the `unsafe` modifier, which is documented gating
    /// behaviour. Test scaffolds and ad-hoc constructions rely on
    /// `permissive()` producing zero diagnostics on every input.
    pub fn permissive() -> Self {
        Self {
            unsafe_allowed: true,
            ffi: true,
            ffi_boundary: verum_common::Text::from("lenient"),
            capability_required: false,
            mls_level: verum_common::Text::from("public"),
            forbid_stdlib_extern: false,
        }
    }

    /// Build from the session's language features.
    pub fn from_features(f: &crate::language_features::SafetyFeatures) -> Self {
        Self {
            unsafe_allowed: f.unsafe_allowed,
            ffi: f.ffi,
            ffi_boundary: f.ffi_boundary.clone(),
            capability_required: f.capability_required,
            mls_level: f.mls_level.clone(),
            forbid_stdlib_extern: f.forbid_stdlib_extern,
        }
    }
}

/// Scan `modules` under the given safety policy and return a
/// diagnostic for every rejected construct. Returns an empty list
/// when the policy permits everything or no violations are present.
pub fn check_safety(modules: &[Module], policy: SafetyPolicy) -> List<Diagnostic> {
    let mut diagnostics = List::new();

    // Surface elevated MLS levels via tracing. The
    // `mls_level` field documents three values (`public`,
    // `secret`, `top_secret`) where higher levels restrict
    // operations, but the ops list is forward-looking infra —
    // no current safety-gate path consults the level at the
    // walk site. Closes the inert-defense pattern by making
    // the elevated-level state observable in logs so an
    // embedder writing `[safety].mls_level = "secret"` sees
    // the level was reached at the gate, even when no
    // operation-level gating fires yet.
    if policy.mls_level.as_str() != "public" {
        tracing::debug!(
            "safety_gate: mls_level = {:?} (elevated MLS) — operation-level \
             restrictions are forward-looking infra; the gate currently logs \
             the level but does not yet restrict specific operations",
            policy.mls_level.as_str()
        );
    }

    // Skip the walk only when EVERY gate is at its permissive
    // default. The `ffi_boundary == "strict"` gate also fires when
    // FFI is allowed (so the policy.ffi == true short-circuit isn't
    // safe on its own); include it in the early-return condition so
    // strict mode actually runs the walk.
    if policy.unsafe_allowed && policy.ffi
        && !policy.capability_required
        && !policy.forbid_stdlib_extern
        && policy.ffi_boundary.as_str() != "strict"
    {
        return diagnostics;
    }
    for module in modules {
        for item in module.items.iter() {
            walk_item(item, &policy, &mut diagnostics);
        }
    }
    diagnostics
}

/// Back-compat: the previous one-flag signature that only checked
/// `unsafe`. New callers should prefer [`check_safety`].
pub fn check_unsafe_usage(
    modules: &[Module],
    unsafe_allowed: bool,
) -> List<Diagnostic> {
    let mut p = SafetyPolicy::permissive();
    p.unsafe_allowed = unsafe_allowed;
    check_safety(modules, p)
}

fn walk_item(item: &Item, policy: &SafetyPolicy, out: &mut List<Diagnostic>) {
    match &item.kind {
        ItemKind::Function(func) => {
            check_function_decl(func, policy, out);
            if let Some(body) = &func.body {
                walk_function_body(body, policy, out);
            }
        }
        ItemKind::Impl(impl_decl) => {
            for impl_item in &impl_decl.items {
                if let ImplItemKind::Function(func) = &impl_item.kind {
                    check_function_decl(func, policy, out);
                    if let Some(body) = &func.body {
                        walk_function_body(body, policy, out);
                    }
                }
            }
        }
        _ => {}
    }
}

/// Gate checks at function declaration level (not body): `unsafe fn`
/// modifier and `extern` / FFI declarations.
fn check_function_decl(
    func: &verum_ast::decl::FunctionDecl,
    policy: &SafetyPolicy,
    out: &mut List<Diagnostic>,
) {
    if !policy.unsafe_allowed && func.is_unsafe {
        out.push(
            DiagnosticBuilder::error()
                .message(format!(
                    "`unsafe fn {}` is not allowed: `[safety] unsafe_allowed` is disabled",
                    func.name.name
                ))
                .span(super::ast_span_to_diagnostic_span(func.span, None))
                .help(
                    "set `unsafe_allowed = true` under `[safety]` in \
                     verum.toml, or remove the `unsafe` modifier",
                )
                .build(),
        );
    }
    if !policy.ffi {
        if let verum_common::Maybe::Some(abi) = &func.extern_abi {
            out.push(
                DiagnosticBuilder::error()
                    .message(format!(
                        "extern function `{}` (abi \"{}\") is not allowed: \
                         `[safety] ffi` is disabled",
                        func.name.name,
                        abi.as_str()
                    ))
                    .span(super::ast_span_to_diagnostic_span(func.span, None))
                    .help(
                        "set `ffi = true` under `[safety]` in verum.toml, \
                         or remove `-Z safety.ffi=false`",
                    )
                    .build(),
            );
        }
    } else if policy.ffi_boundary.as_str() == "strict" {
        // Honour `[safety].ffi_boundary = "strict"`: when FFI is
        // allowed at the project level, require every extern
        // function to carry the `unsafe` modifier so the call-site
        // friction documents the trust boundary. "lenient" mode
        // skips this check (Rust-like — extern fn is implicitly
        // unsafe to call but the declaration itself can omit the
        // modifier).
        //
        // Closes the inert-defense pattern around
        // `SafetyPolicy.ffi_boundary`: pre-fix the field landed on
        // the policy + flowed from manifest but no code path
        // consulted it, so `[safety].ffi_boundary = "lenient"`
        // had no observable effect (and "strict" was the documented
        // default but unenforced).
        if let verum_common::Maybe::Some(abi) = &func.extern_abi {
            if !func.is_unsafe {
                out.push(
                    DiagnosticBuilder::warning()
                        .message(format!(
                            "extern function `{}` (abi \"{}\") should be marked \
                             `unsafe` under `[safety].ffi_boundary = \"strict\"`: \
                             FFI calls cross the trust boundary",
                            func.name.name,
                            abi.as_str()
                        ))
                        .span(super::ast_span_to_diagnostic_span(func.span, None))
                        .help(
                            "add the `unsafe` modifier to the declaration, or \
                             relax `[safety].ffi_boundary` to `\"lenient\"` in \
                             verum.toml",
                        )
                        .build(),
                );
            }
        }
    }
}

fn walk_function_body(body: &FunctionBody, policy: &SafetyPolicy, out: &mut List<Diagnostic>) {
    match body {
        FunctionBody::Block(blk) => walk_block(blk, policy, out),
        FunctionBody::Expr(expr) => walk_expr(expr, policy, out),
    }
}

fn walk_block(block: &Block, policy: &SafetyPolicy, out: &mut List<Diagnostic>) {
    for stmt in &block.stmts {
        walk_stmt(stmt, policy, out);
    }
    if let verum_common::Maybe::Some(e) = &block.expr {
        walk_expr(e, policy, out);
    }
}

fn walk_stmt(stmt: &verum_ast::stmt::Stmt, policy: &SafetyPolicy, out: &mut List<Diagnostic>) {
    match &stmt.kind {
        StmtKind::Expr { expr, .. } => walk_expr(expr, policy, out),
        StmtKind::Let { value, .. } => {
            if let verum_common::Maybe::Some(init) = value {
                walk_expr(init, policy, out);
            }
        }
        _ => {}
    }
}

fn walk_expr(expr: &Expr, policy: &SafetyPolicy, out: &mut List<Diagnostic>) {
    match &expr.kind {
        ExprKind::Unsafe(block) => {
            if !policy.unsafe_allowed {
                out.push(
                    DiagnosticBuilder::error()
                        .message(
                            "`unsafe` blocks are not allowed: \
                             `[safety] unsafe_allowed` is disabled",
                        )
                        .span(super::ast_span_to_diagnostic_span(expr.span, None))
                        .help(
                            "set `unsafe_allowed = true` under `[safety]` in \
                             verum.toml, or remove `-Z safety.unsafe_allowed=false`",
                        )
                        .build(),
                );
            }
            // Recurse — nested unsafe still worth reporting.
            walk_block(block, policy, out);
        }
        ExprKind::Block(block) => walk_block(block, policy, out),
        ExprKind::Async(block) | ExprKind::Meta(block) => walk_block(block, policy, out),
        ExprKind::If { condition, then_branch, else_branch } => {
            for cond in &condition.conditions {
                match cond {
                    ConditionKind::Expr(e) => walk_expr(e, policy, out),
                    ConditionKind::Let { value, .. } => walk_expr(value, policy, out),
                }
            }
            walk_block(then_branch, policy, out);
            if let verum_common::Maybe::Some(else_b) = else_branch {
                walk_expr(else_b, policy, out);
            }
        }
        ExprKind::Match { expr: scrutinee, arms } => {
            walk_expr(scrutinee, policy, out);
            for arm in arms {
                if let verum_common::Maybe::Some(guard) = &arm.guard {
                    walk_expr(guard, policy, out);
                }
                walk_expr(&arm.body, policy, out);
            }
        }
        ExprKind::Loop { body, .. } => walk_block(body, policy, out),
        ExprKind::While { condition, body, .. } => {
            walk_expr(condition, policy, out);
            walk_block(body, policy, out);
        }
        ExprKind::For { iter, body, .. } => {
            walk_expr(iter, policy, out);
            walk_block(body, policy, out);
        }
        ExprKind::Call { func, args, .. } => {
            walk_expr(func, policy, out);
            for a in args {
                walk_expr(a, policy, out);
            }
        }
        ExprKind::Binary { left, right, .. } => {
            walk_expr(left, policy, out);
            walk_expr(right, policy, out);
        }
        ExprKind::Unary { expr, .. } => walk_expr(expr, policy, out),
        ExprKind::Return(maybe) => {
            if let verum_common::Maybe::Some(e) = maybe {
                walk_expr(e, policy, out);
            }
        }
        ExprKind::Paren(inner) => walk_expr(inner, policy, out),
        _ => {
            // Other expression kinds (Literal, Path, Lambda, …) are
            // either leaves or have shapes we don't need to descend
            // into for this check. A miss here is a false negative —
            // the safe failure mode.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::decl::{FunctionDecl, ItemKind};
    use verum_ast::expr::{Block, Expr, ExprKind};
    use verum_ast::{FileId, Span};
    use verum_common::{List, Maybe};

    fn mk_unsafe_expr() -> Expr {
        let blk = Block {
            stmts: List::new(),
            expr: Maybe::None,
            span: Span::dummy(),
        };
        Expr::new(ExprKind::Unsafe(blk), Span::dummy())
    }

    fn mk_module_with_unsafe() -> Module {
        // Build a minimal main fn with { unsafe {} } in its body.
        let inner_block = Block {
            stmts: List::new(),
            expr: Maybe::Some(Box::new(mk_unsafe_expr())),
            span: Span::dummy(),
        };
        let func = FunctionDecl {
            visibility: Default::default(),
            name: verum_ast::ty::Ident::new("main", Span::dummy()),
            generics: List::new(),
            params: List::new(),
            return_type: Maybe::None,
            throws_clause: Maybe::None,
            body: Some(FunctionBody::Block(inner_block)),
            attributes: List::new(),
            is_async: false,
            is_meta: false,
            is_unsafe: false,
            span: Span::dummy(),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            stage_level: 0,
            is_pure: false,
            is_generator: false,
            is_cofix: false,
            is_transparent: false,
            extern_abi: Maybe::None,
            is_variadic: false,
            std_attr: Maybe::None,
            contexts: List::new(),
        };
        let mut items = List::new();
        items.push(Item::new(ItemKind::Function(func), Span::dummy()));
        Module {
            items,
            attributes: List::new(),
            file_id: FileId::new(0),
            span: Span::dummy(),
        }
    }

    #[test]
    fn allowed_yields_no_diagnostics_even_with_unsafe() {
        let module = mk_module_with_unsafe();
        let diags = check_unsafe_usage(&[module], true);
        assert_eq!(diags.len(), 0);
    }

    #[test]
    fn disabled_flags_unsafe_block() {
        let module = mk_module_with_unsafe();
        let diags = check_unsafe_usage(&[module], false);
        assert_eq!(diags.len(), 1, "one unsafe block → one diagnostic");
        let msg = diags.iter().next().unwrap().message();
        assert!(
            msg.contains("unsafe") && msg.contains("[safety]"),
            "diag must point at the config key (got: {})",
            msg
        );
    }

    #[test]
    fn disabled_with_empty_module_yields_nothing() {
        let empty = Module {
            items: List::new(),
            attributes: List::new(),
            file_id: FileId::new(0),
            span: Span::dummy(),
        };
        let diags = check_unsafe_usage(&[empty], false);
        assert_eq!(diags.len(), 0);
    }

    fn mk_module_with_function(
        is_unsafe: bool,
        extern_abi: Maybe<verum_common::Text>,
    ) -> Module {
        let func = FunctionDecl {
            visibility: Default::default(),
            name: verum_ast::ty::Ident::new("native_fn", Span::dummy()),
            generics: List::new(),
            params: List::new(),
            return_type: Maybe::None,
            throws_clause: Maybe::None,
            body: None,
            attributes: List::new(),
            is_async: false,
            is_meta: false,
            is_unsafe,
            span: Span::dummy(),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            stage_level: 0,
            is_pure: false,
            is_generator: false,
            is_cofix: false,
            is_transparent: false,
            extern_abi,
            is_variadic: false,
            std_attr: Maybe::None,
            contexts: List::new(),
        };
        let mut items = List::new();
        items.push(Item::new(ItemKind::Function(func), Span::dummy()));
        Module {
            items,
            attributes: List::new(),
            file_id: FileId::new(0),
            span: Span::dummy(),
        }
    }

    #[test]
    fn unsafe_fn_rejected_when_unsafe_disabled() {
        let module = mk_module_with_function(true, Maybe::None);
        let mut policy = SafetyPolicy::permissive(); policy.unsafe_allowed = false;
        let diags = check_safety(&[module], policy);
        assert_eq!(diags.len(), 1);
        let msg = diags.iter().next().unwrap().message();
        assert!(msg.contains("unsafe fn"), "got: {}", msg);
        assert!(msg.contains("[safety]"), "got: {}", msg);
    }

    #[test]
    fn ffi_fn_rejected_when_ffi_disabled() {
        let module = mk_module_with_function(
            false,
            Maybe::Some(verum_common::Text::from("C")),
        );
        let mut policy = SafetyPolicy::permissive(); policy.ffi = false;
        let diags = check_safety(&[module], policy);
        assert_eq!(diags.len(), 1);
        let msg = diags.iter().next().unwrap().message();
        assert!(msg.contains("extern"), "got: {}", msg);
        assert!(msg.contains("[safety] ffi"), "got: {}", msg);
        assert!(msg.contains("C"), "abi name should appear: {}", msg);
    }

    #[test]
    fn ffi_permissive_policy_allows_extern_fn() {
        let module = mk_module_with_function(
            false,
            Maybe::Some(verum_common::Text::from("C")),
        );
        let policy = SafetyPolicy::permissive();
        let diags = check_safety(&[module], policy);
        assert_eq!(diags.len(), 0);
    }

    #[test]
    fn combined_unsafe_and_ffi_violations_both_reported() {
        // unsafe fn + extern abi — both disabled → 2 diagnostics.
        let module = mk_module_with_function(
            true,
            Maybe::Some(verum_common::Text::from("C")),
        );
        let mut policy = SafetyPolicy::permissive(); policy.unsafe_allowed = false; policy.ffi = false;
        let diags = check_safety(&[module], policy);
        assert_eq!(diags.len(), 2, "both gates should fire independently");
    }

    #[test]
    fn ffi_strict_mode_warns_on_extern_without_unsafe() {
        // Pin the inert-defense closure for `SafetyPolicy.ffi_boundary`.
        // Pre-fix the field was set on the policy + flowed from manifest
        // but no code path consulted it, so `[safety].ffi_boundary =
        // "strict"` had no observable effect on the safety walk.
        //
        // With the wire-up, an extern function declared without `unsafe`
        // surfaces a warning suggesting the modifier — pinning the
        // strict-mode contract end-to-end.
        let module = mk_module_with_function(
            false,
            Maybe::Some(verum_common::Text::from("C")),
        );
        let mut policy = SafetyPolicy::permissive();
        policy.ffi_boundary = verum_common::Text::from("strict");
        let diags = check_safety(&[module], policy);
        assert_eq!(diags.len(), 1, "strict mode must warn on extern without unsafe");
    }

    #[test]
    fn ffi_strict_mode_quiet_on_extern_with_unsafe() {
        // Pin the inverse: extern function WITH `unsafe` modifier
        // satisfies strict mode, no warning fires.
        let module = mk_module_with_function(
            true,
            Maybe::Some(verum_common::Text::from("C")),
        );
        let mut policy = SafetyPolicy::permissive();
        policy.ffi_boundary = verum_common::Text::from("strict");
        let diags = check_safety(&[module], policy);
        assert_eq!(
            diags.len(),
            0,
            "strict mode + unsafe extern must produce no diagnostics"
        );
    }

    #[test]
    fn ffi_lenient_mode_quiet_on_extern_without_unsafe() {
        // Pin the lenient mode: extern function without `unsafe`
        // is allowed silently when ffi_boundary = "lenient".
        let module = mk_module_with_function(
            false,
            Maybe::Some(verum_common::Text::from("C")),
        );
        let mut policy = SafetyPolicy::permissive();
        policy.ffi_boundary = verum_common::Text::from("lenient");
        let diags = check_safety(&[module], policy);
        assert_eq!(
            diags.len(),
            0,
            "lenient mode must allow extern without unsafe modifier"
        );
    }
}
