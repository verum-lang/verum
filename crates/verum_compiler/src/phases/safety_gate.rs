//! Pre-typecheck safety-feature gates.
//!

//! Walks the parsed AST looking for language constructs that are
//! disabled by the current `[safety]` feature set, emitting clean
//! diagnostics before type-checking runs. Keeping these checks in
//! their own walker avoids threading feature flags through the
//! ~52K-line `TypeChecker` and keeps the gate logic auditable.
//!

//! Currently gates:
//!  - `unsafe { … }` expressions when `safety.unsafe_allowed = false`.

use verum_ast::decl::{FunctionBody, ImplItemKind, Item, ItemKind};
use verum_ast::expr::{Block, ConditionKind};
use verum_ast::stmt::StmtKind;
use verum_ast::{Expr, ExprKind, Module};
use verum_common::List;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};

/// Policy for the safety-gate walker.
///

/// Each flag maps 1:1 to a `[safety]` field in `Verum.toml`. When a
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

    // Surface elevated MLS levels via tracing for observability.
    // Operation-level enforcement runs in `walk_item` below
    // (Phase 1 of #266 — surface gate). Pre-fix the gate
    // logged the level but did not enforce.
    if policy.mls_level.as_str() != "public" {
        tracing::debug!(
            "safety_gate: mls_level = {:?} (elevated MLS) — Phase 1 surface gate \
             active: extern fn / unsafe fn / unsafe blocks must carry \
             @classification(secret|top_secret) matching the floor",
            policy.mls_level.as_str()
        );
    }

    // Skip the walk only when EVERY gate is at its permissive
    // default. The `ffi_boundary == "strict"` gate also fires when
    // FFI is allowed (so the policy.ffi == true short-circuit isn't
    // safe on its own); include it in the early-return condition so
    // strict mode actually runs the walk.
    //

    // mls_level != "public" also requires the walk to fire — the
    // Phase 1 gate (#266) inspects every extern fn / unsafe fn /
    // unsafe block for an @classification annotation matching the
    // manifest floor. Non-public mls_level is the marker.
    if policy.unsafe_allowed
        && policy.ffi
        && !policy.capability_required
        && !policy.forbid_stdlib_extern
        && policy.ffi_boundary.as_str() != "strict"
        && policy.mls_level.as_str() == "public"
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
pub fn check_unsafe_usage(modules: &[Module], unsafe_allowed: bool) -> List<Diagnostic> {
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

/// MLS classification level — re-exported from
/// `verum_common::mls::MlsLevel` (#282 Phase 2a).
///

/// Pre-#282 this enum was private to safety_gate. Promoting it to
/// the shared layer lets the type checker (Phase 2b) and the
/// context system (Phase 3) consume the same lattice without
/// re-defining it. Phase 1 (#266 surface gate) continues to use
/// only `subsumes` / `from_manifest_str` — the lattice operations
/// (`join`, `meet`) are reserved for downstream Phase 2b/3 work.
use verum_common::mls::MlsLevel;

/// Inspect an attribute list for `@classification(<level>)`. Returns
/// the highest classification declared (so a function carrying
/// `@classification(top_secret)` AND `@classification(secret)` —
/// pathological but legal AST — produces TopSecret). Returns
/// `MlsLevel::Public` when no classification attribute is present
/// (i.e., the function inherits the public floor).
fn read_classification(attrs: &List<verum_ast::attr::Attribute>) -> MlsLevel {
    let mut found = MlsLevel::Public;
    for attr in attrs.iter() {
        if !attr.is_named("classification") {
            continue;
        }
        if let verum_common::Maybe::Some(args) = &attr.args {
            for arg in args.iter() {
                // The classification level is the first identifier
                // argument: `@classification(secret)` or
                // `@classification(top_secret)`. Anything else
                // (string literals, malformed args) is ignored —
                // those failures are the parser's domain.
                if let verum_ast::expr::ExprKind::Path(path) = &arg.kind {
                    if let Some(ident) = path.as_ident() {
                        let parsed = MlsLevel::from_manifest_str(ident.as_str());
                        if parsed > found {
                            found = parsed;
                        }
                    }
                }
            }
        }
    }
    found
}

/// Find the highest-classified parameter on `func`, returning
/// `Some((name, level, span))` for the maximally-classified one
/// when at least one parameter is non-Public, or `None` when every
/// parameter is unclassified. Used by the Phase 2b diagnostic to
/// point users at the SOURCE of the parameter classification
/// (otherwise the error would just say "the function" without
/// indicating which parameter triggered the requirement).
fn highest_classified_param(
    func: &verum_ast::decl::FunctionDecl,
) -> Option<(verum_common::Text, MlsLevel, verum_common::span::Span)> {
    let mut best: Option<(verum_common::Text, MlsLevel, verum_common::span::Span)> = None;
    for param in func.params.iter() {
        let level = read_classification(&param.attributes);
        if level == MlsLevel::Public {
            continue;
        }
        let name = match &param.kind {
            verum_ast::decl::FunctionParamKind::Regular { pattern, .. } => pattern_to_name(pattern),
            _ => verum_common::Text::from("self"),
        };
        match &best {
            Some((_, prev, _)) if *prev >= level => {}
            _ => {
                best = Some((name, level, param.span));
            }
        }
    }
    best
}

fn pattern_to_name(p: &verum_ast::pattern::Pattern) -> verum_common::Text {
    use verum_ast::pattern::PatternKind;
    match &p.kind {
        PatternKind::Ident { name, .. } => name.name.clone(),
        _ => verum_common::Text::from("<param>"),
    }
}

/// MLS surface gate (Phase 1 of #266 + Phase 2b of #288). Emits a
/// diagnostic when:
///  1. `[safety].mls_level` is non-`"public"` (the manifest floor),
///  2. the function is FFI (extern_abi present), `unsafe fn`, OR
///  carries any classified parameter (Phase 2b addition), AND
///  3. the function's effective classification (its own
///  `@classification` joined with every parameter's
///  `@classification`) is < the manifest floor.
///

/// Phase 1 covered the call-site friction layer for dangerous
/// declarations. Phase 2b extends the trigger to functions that
/// merely RECEIVE classified data (a Secret-classified parameter
/// is a contract that the function handles classified data
/// appropriately — it must itself be classified). Full type-
/// level taint propagation through Pi-types remains Phase 2b-full
/// at `verum_types::infer`.
/// Default low-classification sink registry (#283 Phase 3a).
///

/// These context names are recognized as sinks where classified
/// data leaks observably out of the program (logs, files, network
/// packets). When a function with a Secret-or-higher classification
/// `using` one of these contexts, the surface gate emits a leak
/// warning unless the function is explicitly marked `@declassify`.
///

/// The list is conservative — only contexts whose semantic is
/// "this is publicly observable output" are sinks. Pure-compute
/// contexts (Database queries that return Secret data, validation
/// services) are NOT sinks; they're classified-data CONSUMERS.
///

/// The registry is hardcoded at the prefix level — any context
/// whose final path segment matches one of these is a sink.
/// Phase 3 full-form will extend this with manifest-driven custom
/// sink declarations under `[safety].mls_sinks = ["MyAuditLog"]`.
const DEFAULT_LOW_CLASSIFICATION_SINKS: &[&str] = &[
    "Logger",
    "FS",
    "FileSystem",
    "Network",
    "Stdout",
    "Stderr",
    "Tracing",
    "Telemetry",
];

/// Determine whether a `ContextRequirement` references a known
/// low-classification sink (#283 Phase 3a). Matches against the
/// final identifier segment so `core.io.Logger` and
/// `my_lib.audit.Logger` both register as Logger sinks.
fn is_low_classification_sink(ctx: &verum_ast::context::ContextRequirement) -> bool {
    if ctx.is_negative {
        return false;
    }
    let last = ctx.path.last_segment_name();
    DEFAULT_LOW_CLASSIFICATION_SINKS
        .iter()
        .any(|sink| *sink == last)
}

/// Check if a function carries the `@declassify` attribute, which
/// is the explicit escape hatch for classified data flowing into
/// low-classification sinks (#283). Without this attribute the
/// surface gate rejects the leak; with it the user accepts
/// responsibility (the value's classification is shed at the
/// `@declassify` boundary).
fn has_declassify_attr(attrs: &List<verum_ast::attr::Attribute>) -> bool {
    attrs.iter().any(|a| a.is_named("declassify"))
}

/// MLS Phase 3a sink-detection (#283). Emits a diagnostic when a
/// function:
///  1. carries a classified parameter (Phase 2b trigger), AND
///  2. uses a low-classification sink context (Logger, FS,
///  Network, …), AND
///  3. is NOT marked `@declassify`.
///

/// This is the surface-level information-flow check: classified
/// data + observable sink + no explicit declassification = leak.
/// Full type-level taint propagation (where every classified value
/// is tracked through let-bindings and rejected at sink boundaries)
/// remains Phase 3-full follow-up.
fn check_mls_sink_leak(
    func: &verum_ast::decl::FunctionDecl,
    policy: &SafetyPolicy,
    out: &mut List<Diagnostic>,
) {
    let floor = MlsLevel::from_manifest_str(policy.mls_level.as_str());
    if floor == MlsLevel::Public {
        return;
    }
    let highest_param = highest_classified_param(func);
    let param_max = match highest_param {
        Some((_, lvl, _)) => lvl,
        None => return, // No classified params → no leak surface.
    };
    if has_declassify_attr(&func.attributes) {
        return; // Explicit escape hatch.
    }
    // Find any low-classification sink in the function's `using`
    // contexts. The first match drives the diagnostic.
    let sink = func
        .contexts
        .iter()
        .find(|ctx| is_low_classification_sink(ctx));
    let sink = match sink {
        Some(s) => s,
        None => return,
    };
    out.push(
        DiagnosticBuilder::error()
            .message(format!(
                "function `{}` leaks {}-classified data into context \
                 `{}` (a low-classification sink)",
                func.name.name,
                param_max.as_manifest_str(),
                sink.path.last_segment_name(),
            ))
            .span(super::ast_span_to_diagnostic_span(func.span, None))
            .help(
                "either remove the classified parameter, drop the sink \
                 context from `using [...]`, or mark the function with \
                 `@declassify` to explicitly accept the leak",
            )
            .build(),
    );
}

fn check_mls_classification(
    func: &verum_ast::decl::FunctionDecl,
    policy: &SafetyPolicy,
    out: &mut List<Diagnostic>,
) {
    let floor = MlsLevel::from_manifest_str(policy.mls_level.as_str());
    if floor == MlsLevel::Public {
        return; // Hot path: no manifest floor, no gate.
    }
    let is_ffi = matches!(&func.extern_abi, verum_common::Maybe::Some(_));
    let is_unsafe_decl = func.is_unsafe;
    let highest_param = highest_classified_param(func);
    let has_classified_param = highest_param.is_some();
    // Phase 1 trigger: dangerous-construct declarations (ffi /
    // unsafe). Phase 2b trigger: classified parameters without
    // a function-level @classification matching them.
    if !is_ffi && !is_unsafe_decl && !has_classified_param {
        return;
    }

    // Function-level declared classification. The function MUST
    // explicitly opt in via @classification — Phase 2b's contract
    // is that classified data flowing through a function requires
    // an explicit handler classification, not implicit inheritance
    // from a parameter.
    let func_declared = read_classification(&func.attributes);
    let param_max = highest_param
        .as_ref()
        .map(|(_, lvl, _)| *lvl)
        .unwrap_or(MlsLevel::Public);
    // The minimum acceptable function-level classification is the
    // join of: the manifest floor (when ffi/unsafe), and the
    // highest param classification (regardless of trigger). The
    // function MUST be at least this high — whichever trigger
    // fired.
    let manifest_required = if is_ffi || is_unsafe_decl {
        floor
    } else {
        MlsLevel::Public
    };
    let required = manifest_required.join(param_max);
    if func_declared >= required {
        return;
    }

    let trigger = if is_ffi {
        "extern"
    } else if is_unsafe_decl {
        "unsafe"
    } else {
        "classified-param"
    };
    let mut message = format!(
        "{trigger} function `{}` requires `@classification({})` \
         (or higher) under `[safety].mls_level = \"{}\"`",
        func.name.name,
        required.as_manifest_str(),
        floor.as_manifest_str(),
    );
    if let Some((param_name, param_level, _)) = &highest_param {
        message.push_str(&format!(
            " — parameter `{}` is classified `{}`",
            param_name.as_str(),
            param_level.as_manifest_str(),
        ));
    }
    out.push(
        DiagnosticBuilder::error()
            .message(message)
            .span(super::ast_span_to_diagnostic_span(func.span, None))
            .help(format!(
                "add `@classification({})` to the function declaration, \
                 or relax `[safety].mls_level` to `\"public\"` in Verum.toml",
                required.as_manifest_str(),
            ))
            .build(),
    );
}

/// Gate checks at function declaration level (not body): `unsafe fn`
/// modifier and `extern` / FFI declarations.
fn check_function_decl(
    func: &verum_ast::decl::FunctionDecl,
    policy: &SafetyPolicy,
    out: &mut List<Diagnostic>,
) {
    check_mls_classification(func, policy, out);
    check_mls_sink_leak(func, policy, out);
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
                     Verum.toml, or remove the `unsafe` modifier",
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
                        "set `ffi = true` under `[safety]` in Verum.toml, \
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
                             Verum.toml",
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
                             Verum.toml, or remove `-Z safety.unsafe_allowed=false`",
                        )
                        .build(),
                );
            }
            // Recurse — nested unsafe still worth reporting.
            walk_block(block, policy, out);
        }
        ExprKind::Block(block) => walk_block(block, policy, out),
        ExprKind::Async(block) | ExprKind::Meta(block) => walk_block(block, policy, out),
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
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
        ExprKind::Match {
            expr: scrutinee,
            arms,
        } => {
            walk_expr(scrutinee, policy, out);
            for arm in arms {
                if let verum_common::Maybe::Some(guard) = &arm.guard {
                    walk_expr(guard, policy, out);
                }
                walk_expr(&arm.body, policy, out);
            }
        }
        ExprKind::Loop { body, .. } => walk_block(body, policy, out),
        ExprKind::While {
            condition, body, ..
        } => {
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

    fn mk_module_with_function(is_unsafe: bool, extern_abi: Maybe<verum_common::Text>) -> Module {
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
        let mut policy = SafetyPolicy::permissive();
        policy.unsafe_allowed = false;
        let diags = check_safety(&[module], policy);
        assert_eq!(diags.len(), 1);
        let msg = diags.iter().next().unwrap().message();
        assert!(msg.contains("unsafe fn"), "got: {}", msg);
        assert!(msg.contains("[safety]"), "got: {}", msg);
    }

    #[test]
    fn ffi_fn_rejected_when_ffi_disabled() {
        let module = mk_module_with_function(false, Maybe::Some(verum_common::Text::from("C")));
        let mut policy = SafetyPolicy::permissive();
        policy.ffi = false;
        let diags = check_safety(&[module], policy);
        assert_eq!(diags.len(), 1);
        let msg = diags.iter().next().unwrap().message();
        assert!(msg.contains("extern"), "got: {}", msg);
        assert!(msg.contains("[safety] ffi"), "got: {}", msg);
        assert!(msg.contains("C"), "abi name should appear: {}", msg);
    }

    #[test]
    fn ffi_permissive_policy_allows_extern_fn() {
        let module = mk_module_with_function(false, Maybe::Some(verum_common::Text::from("C")));
        let policy = SafetyPolicy::permissive();
        let diags = check_safety(&[module], policy);
        assert_eq!(diags.len(), 0);
    }

    #[test]
    fn combined_unsafe_and_ffi_violations_both_reported() {
        // unsafe fn + extern abi — both disabled → 2 diagnostics.
        let module = mk_module_with_function(true, Maybe::Some(verum_common::Text::from("C")));
        let mut policy = SafetyPolicy::permissive();
        policy.unsafe_allowed = false;
        policy.ffi = false;
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
        let module = mk_module_with_function(false, Maybe::Some(verum_common::Text::from("C")));
        let mut policy = SafetyPolicy::permissive();
        policy.ffi_boundary = verum_common::Text::from("strict");
        let diags = check_safety(&[module], policy);
        assert_eq!(
            diags.len(),
            1,
            "strict mode must warn on extern without unsafe"
        );
    }

    #[test]
    fn ffi_strict_mode_quiet_on_extern_with_unsafe() {
        // Pin the inverse: extern function WITH `unsafe` modifier
        // satisfies strict mode, no warning fires.
        let module = mk_module_with_function(true, Maybe::Some(verum_common::Text::from("C")));
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
        let module = mk_module_with_function(false, Maybe::Some(verum_common::Text::from("C")));
        let mut policy = SafetyPolicy::permissive();
        policy.ffi_boundary = verum_common::Text::from("lenient");
        let diags = check_safety(&[module], policy);
        assert_eq!(
            diags.len(),
            0,
            "lenient mode must allow extern without unsafe modifier"
        );
    }

    // ============================================================
    // [safety].mls_level Phase 1 surface gate pin tests (#266).
    // ============================================================

    /// Build a function with attributes (used for @classification tests).
    fn mk_module_with_function_attrs(
        is_unsafe: bool,
        extern_abi: Maybe<verum_common::Text>,
        attributes: List<verum_ast::attr::Attribute>,
    ) -> Module {
        let func = FunctionDecl {
            visibility: Default::default(),
            name: verum_ast::ty::Ident::new("native_fn", Span::dummy()),
            generics: List::new(),
            params: List::new(),
            return_type: Maybe::None,
            throws_clause: Maybe::None,
            body: None,
            attributes,
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

    /// Build `@classification(<level>)` as an Attribute.
    fn mk_classification_attr(level: &str) -> verum_ast::attr::Attribute {
        let path = verum_ast::ty::Path::single(verum_ast::ty::Ident::new(level, Span::dummy()));
        let arg = Expr::new(ExprKind::Path(path), Span::dummy());
        let mut args = List::new();
        args.push(arg);
        verum_ast::attr::Attribute::new(
            verum_common::Text::from("classification"),
            Maybe::Some(args),
            Span::dummy(),
        )
    }

    #[test]
    fn mls_public_does_not_gate_unannotated_extern() {
        // Pin: when mls_level == "public" (the default), the gate is
        // a no-op — extern fn without @classification is allowed.
        let module = mk_module_with_function(
            true, // unsafe
            Maybe::Some(verum_common::Text::from("C")),
        );
        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("public");
        let diags = check_safety(&[module], policy);
        let mls_diags: Vec<_> = diags
            .iter()
            .filter(|d| {
                let msg = d.message().to_string();
                msg.contains("@classification")
            })
            .collect();
        assert_eq!(
            mls_diags.len(),
            0,
            "public mls_level must not emit @classification diagnostics"
        );
    }

    #[test]
    fn mls_secret_rejects_unannotated_extern() {
        // Pin: under mls_level = "secret", an extern fn without
        // @classification triggers an error citing the manifest.
        let module = mk_module_with_function(false, Maybe::Some(verum_common::Text::from("C")));
        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("secret");
        let diags = check_safety(&[module], policy);
        let mls_diags: Vec<_> = diags
            .iter()
            .filter(|d| {
                let msg = d.message().to_string();
                msg.contains("@classification")
            })
            .collect();
        assert_eq!(
            mls_diags.len(),
            1,
            "secret mls_level must reject unannotated extern; got {} diags",
            mls_diags.len()
        );
    }

    #[test]
    fn mls_secret_rejects_unannotated_unsafe_fn() {
        // Pin: same gate fires on `unsafe fn` declarations
        // (regardless of FFI status).
        let module = mk_module_with_function(true, Maybe::None);
        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("secret");
        let diags = check_safety(&[module], policy);
        let mls_diags: Vec<_> = diags
            .iter()
            .filter(|d| {
                let msg = d.message().to_string();
                msg.contains("@classification") && msg.contains("unsafe")
            })
            .collect();
        assert_eq!(
            mls_diags.len(),
            1,
            "unsafe fn under mls=secret must require @classification"
        );
    }

    #[test]
    fn mls_secret_accepts_secret_classified_extern() {
        // Pin: properly classified extern fn passes the gate cleanly.
        let mut attrs = List::new();
        attrs.push(mk_classification_attr("secret"));
        let module =
            mk_module_with_function_attrs(false, Maybe::Some(verum_common::Text::from("C")), attrs);
        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("secret");
        let diags = check_safety(&[module], policy);
        let mls_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message().to_string().contains("@classification"))
            .collect();
        assert_eq!(
            mls_diags.len(),
            0,
            "@classification(secret) must satisfy mls_level=secret"
        );
    }

    #[test]
    fn mls_secret_accepts_top_secret_classified_extern() {
        // Pin: a HIGHER classification satisfies a lower floor —
        // top_secret-classified function passes mls=secret gate.
        // This is the lattice-monotonicity property.
        let mut attrs = List::new();
        attrs.push(mk_classification_attr("top_secret"));
        let module =
            mk_module_with_function_attrs(false, Maybe::Some(verum_common::Text::from("C")), attrs);
        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("secret");
        let diags = check_safety(&[module], policy);
        let mls_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message().to_string().contains("@classification"))
            .collect();
        assert_eq!(
            mls_diags.len(),
            0,
            "@classification(top_secret) must satisfy lower floor secret"
        );
    }

    #[test]
    fn mls_top_secret_rejects_secret_classified_extern() {
        // Pin: the inverse — a LOWER classification does NOT satisfy
        // a higher floor. secret-classified function fails mls=
        // top_secret gate.
        let mut attrs = List::new();
        attrs.push(mk_classification_attr("secret"));
        let module =
            mk_module_with_function_attrs(false, Maybe::Some(verum_common::Text::from("C")), attrs);
        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("top_secret");
        let diags = check_safety(&[module], policy);
        let mls_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message().to_string().contains("@classification"))
            .collect();
        assert_eq!(
            mls_diags.len(),
            1,
            "@classification(secret) must fail higher floor top_secret"
        );
    }

    #[test]
    fn mls_secret_does_not_gate_safe_pure_function() {
        // Pin: safe (non-unsafe, non-FFI) functions are unaffected by
        // the gate even under elevated mls_level. No false positives
        // on ordinary code.
        let module = mk_module_with_function(false, Maybe::None);
        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("secret");
        let diags = check_safety(&[module], policy);
        let mls_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message().to_string().contains("@classification"))
            .collect();
        assert_eq!(
            mls_diags.len(),
            0,
            "safe non-FFI function must not trigger MLS gate"
        );
    }

    // ============================================================
    // [safety].mls_level Phase 2b parameter-level pin tests (#288).
    //

    // Phase 2b extends the Phase 1 surface gate to detect functions
    // that RECEIVE classified data via parameter-level
    // `@classification` annotations. The function's effective
    // classification floor is the JOIN of its own attribute and
    // every parameter's attribute (uses the lattice from #282
    // Phase 2a).
    // ============================================================

    /// Build a function with classified parameters.
    fn mk_module_with_classified_param(
        is_unsafe: bool,
        extern_abi: Maybe<verum_common::Text>,
        param_classification: Maybe<&'static str>,
    ) -> Module {
        use verum_ast::pattern::{Pattern, PatternKind};

        let mut param_attrs = List::new();
        if let Maybe::Some(level) = param_classification {
            param_attrs.push(mk_classification_attr(level));
        }

        let param = verum_ast::decl::FunctionParam {
            kind: verum_ast::decl::FunctionParamKind::Regular {
                pattern: Pattern {
                    kind: PatternKind::Ident {
                        by_ref: false,
                        mutable: false,
                        name: verum_ast::ty::Ident::new("data", Span::dummy()),
                        subpattern: Maybe::None,
                    },
                    span: Span::dummy(),
                },
                ty: verum_ast::ty::Type {
                    kind: verum_ast::ty::TypeKind::Path(verum_ast::ty::Path::single(
                        verum_ast::ty::Ident::new("Int", Span::dummy()),
                    )),
                    span: Span::dummy(),
                },
                default_value: Maybe::None,
            },
            attributes: param_attrs,
            span: Span::dummy(),
        };
        let mut params = List::new();
        params.push(param);

        let func = FunctionDecl {
            visibility: Default::default(),
            name: verum_ast::ty::Ident::new("handler", Span::dummy()),
            generics: List::new(),
            params,
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
    fn mls_phase2b_secret_param_without_classified_function_rejected() {
        // Pin: a function with a Secret-classified parameter must
        // itself be at least Secret — otherwise Secret data flows
        // through an unclassified function body. The Phase 2b
        // surface gate catches this even when the function is
        // neither extern nor unsafe.
        let module = mk_module_with_classified_param(false, Maybe::None, Maybe::Some("secret"));
        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("secret");
        let diags = check_safety(&[module], policy);
        let mls_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message().to_string().contains("@classification"))
            .collect();
        assert_eq!(
            mls_diags.len(),
            1,
            "classified-param-only trigger must fire under non-public mls"
        );
        let msg = mls_diags[0].message().to_string();
        assert!(msg.contains("classified-param"), "got: {}", msg);
        assert!(
            msg.contains("data"),
            "diagnostic must name the offending parameter; got: {}",
            msg
        );
        assert!(msg.contains("secret"), "got: {}", msg);
    }

    #[test]
    fn mls_phase2b_param_classification_is_inherited_to_function() {
        // Pin: when the function is itself @classification(secret)
        // AND has a Secret-classified param, both consistency
        // checks pass — no diagnostic. The function's effective
        // floor (join of own + params) is Secret, which subsumes
        // the manifest floor.
        let mut func_attrs = List::new();
        func_attrs.push(mk_classification_attr("secret"));

        // Build directly so we can attach the function-level attr.
        let mut module = mk_module_with_classified_param(false, Maybe::None, Maybe::Some("secret"));
        if let ItemKind::Function(ref mut f) = module.items[0].kind {
            f.attributes = func_attrs;
        }

        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("secret");
        let diags = check_safety(&[module], policy);
        let mls_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message().to_string().contains("@classification"))
            .collect();
        assert_eq!(
            mls_diags.len(),
            0,
            "secret param + secret function must pass; got {} diags",
            mls_diags.len()
        );
    }

    #[test]
    fn mls_phase2b_top_secret_param_passes_secret_floor() {
        // Pin: lattice JOIN — TopSecret-classified param produces
        // an effective floor of TopSecret (≥ Secret manifest floor)
        // even when the function itself has no @classification.
        // No: this would still need the FUNCTION to be classified
        // (since the param's TopSecret floor must be matched).
        // Actually: with param=TopSecret AND function=TopSecret,
        // both pass. Verify with explicit @classification(top_secret)
        // on the function.
        let mut func_attrs = List::new();
        func_attrs.push(mk_classification_attr("top_secret"));

        let mut module =
            mk_module_with_classified_param(false, Maybe::None, Maybe::Some("top_secret"));
        if let ItemKind::Function(ref mut f) = module.items[0].kind {
            f.attributes = func_attrs;
        }

        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("secret");
        let diags = check_safety(&[module], policy);
        let mls_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message().to_string().contains("@classification"))
            .collect();
        assert_eq!(
            mls_diags.len(),
            0,
            "top_secret function + top_secret param must satisfy secret floor"
        );
    }

    #[test]
    fn mls_phase2b_unclassified_param_does_not_trigger() {
        // Pin: parameters WITHOUT @classification don't trigger
        // the Phase 2b gate. The function body is still unsafe-or-
        // ffi-checked under Phase 1 rules.
        let module = mk_module_with_classified_param(
            false,
            Maybe::None,
            Maybe::None, // No classification on param.
        );
        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("secret");
        let diags = check_safety(&[module], policy);
        let mls_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message().to_string().contains("@classification"))
            .collect();
        assert_eq!(
            mls_diags.len(),
            0,
            "unclassified params must not trigger Phase 2b gate"
        );
    }

    #[test]
    fn mls_phase2b_lattice_join_uses_max_param() {
        // Pin: when multiple params have different classifications,
        // the lattice JOIN takes the highest. The diagnostic
        // identifies the highest-classified param.
        // Build two-param function: one Secret, one TopSecret.
        let mut module = mk_module_with_classified_param(false, Maybe::None, Maybe::Some("secret"));
        // Add a second param at TopSecret.
        if let ItemKind::Function(ref mut f) = module.items[0].kind {
            use verum_ast::pattern::{Pattern, PatternKind};
            let mut param_attrs = List::new();
            param_attrs.push(mk_classification_attr("top_secret"));
            let param2 = verum_ast::decl::FunctionParam {
                kind: verum_ast::decl::FunctionParamKind::Regular {
                    pattern: Pattern {
                        kind: PatternKind::Ident {
                            by_ref: false,
                            mutable: false,
                            name: verum_ast::ty::Ident::new("ts_data", Span::dummy()),
                            subpattern: Maybe::None,
                        },
                        span: Span::dummy(),
                    },
                    ty: verum_ast::ty::Type {
                        kind: verum_ast::ty::TypeKind::Path(verum_ast::ty::Path::single(
                            verum_ast::ty::Ident::new("Int", Span::dummy()),
                        )),
                        span: Span::dummy(),
                    },
                    default_value: Maybe::None,
                },
                attributes: param_attrs,
                span: Span::dummy(),
            };
            f.params.push(param2);
        }

        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("secret");
        let diags = check_safety(&[module], policy);
        let mls_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message().to_string().contains("@classification"))
            .collect();
        assert_eq!(
            mls_diags.len(),
            1,
            "two-param case must fire one diagnostic"
        );
        let msg = mls_diags[0].message().to_string();
        // The highest-classified param (ts_data, TopSecret) should
        // be cited in the diagnostic.
        assert!(
            msg.contains("ts_data"),
            "diagnostic must cite highest-classified param; got: {}",
            msg
        );
        assert!(msg.contains("top_secret"), "got: {}", msg);
    }

    // ============================================================
    // [safety].mls_level Phase 3a sink-detection pin tests (#283).
    //

    // Phase 3a: classified data + low-classification sink (Logger,
    // FS, Network, …) + no @declassify = leak diagnostic.
    // ============================================================

    /// Build a function with a classified param AND a `using
    /// [<sink>]` context for sink-leak tests.
    fn mk_module_with_classified_param_and_context(
        param_classification: Maybe<&'static str>,
        context_name: &str,
        function_attrs: List<verum_ast::attr::Attribute>,
    ) -> Module {
        use verum_ast::context::ContextRequirement;
        use verum_ast::pattern::{Pattern, PatternKind};

        let mut param_attrs = List::new();
        if let Maybe::Some(level) = param_classification {
            param_attrs.push(mk_classification_attr(level));
        }

        let param = verum_ast::decl::FunctionParam {
            kind: verum_ast::decl::FunctionParamKind::Regular {
                pattern: Pattern {
                    kind: PatternKind::Ident {
                        by_ref: false,
                        mutable: false,
                        name: verum_ast::ty::Ident::new("data", Span::dummy()),
                        subpattern: Maybe::None,
                    },
                    span: Span::dummy(),
                },
                ty: verum_ast::ty::Type {
                    kind: verum_ast::ty::TypeKind::Path(verum_ast::ty::Path::single(
                        verum_ast::ty::Ident::new("Int", Span::dummy()),
                    )),
                    span: Span::dummy(),
                },
                default_value: Maybe::None,
            },
            attributes: param_attrs,
            span: Span::dummy(),
        };
        let mut params = List::new();
        params.push(param);

        let ctx_req = ContextRequirement::simple(
            verum_ast::ty::Path::single(verum_ast::ty::Ident::new(context_name, Span::dummy())),
            List::new(),
            Span::dummy(),
        );
        let mut contexts = List::new();
        contexts.push(ctx_req);

        let func = FunctionDecl {
            visibility: Default::default(),
            name: verum_ast::ty::Ident::new("logger_handler", Span::dummy()),
            generics: List::new(),
            params,
            return_type: Maybe::None,
            throws_clause: Maybe::None,
            body: None,
            attributes: function_attrs,
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
            contexts,
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

    /// Helper: build a `@declassify` attribute (no args needed).
    fn mk_declassify_attr() -> verum_ast::attr::Attribute {
        verum_ast::attr::Attribute::simple(verum_common::Text::from("declassify"), Span::dummy())
    }

    #[test]
    fn mls_phase3a_secret_param_into_logger_rejected() {
        // Pin: secret-classified param + Logger sink + no
        // @declassify = leak diagnostic.
        let mut func_attrs = List::new();
        func_attrs.push(mk_classification_attr("secret"));
        let module = mk_module_with_classified_param_and_context(
            Maybe::Some("secret"),
            "Logger",
            func_attrs,
        );
        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("secret");
        let diags = check_safety(&[module], policy);
        let leak_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message().to_string().contains("low-classification sink"))
            .collect();
        assert_eq!(
            leak_diags.len(),
            1,
            "secret param + Logger sink must trigger leak diagnostic"
        );
        let msg = leak_diags[0].message().to_string();
        assert!(msg.contains("Logger"), "got: {}", msg);
        assert!(msg.contains("secret"), "got: {}", msg);
    }

    #[test]
    fn mls_phase3a_declassify_escape_hatch_silences_leak() {
        // Pin: @declassify on the function declaration acts as the
        // explicit escape hatch — the leak is suppressed. User
        // accepted responsibility for the boundary.
        let mut func_attrs = List::new();
        func_attrs.push(mk_classification_attr("secret"));
        func_attrs.push(mk_declassify_attr());
        let module = mk_module_with_classified_param_and_context(
            Maybe::Some("secret"),
            "Logger",
            func_attrs,
        );
        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("secret");
        let diags = check_safety(&[module], policy);
        let leak_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message().to_string().contains("low-classification sink"))
            .collect();
        assert_eq!(
            leak_diags.len(),
            0,
            "@declassify must suppress leak diagnostic"
        );
    }

    #[test]
    fn mls_phase3a_unclassified_param_does_not_trigger() {
        // Pin: function using Logger sink WITHOUT classified
        // params is fine — no leak surface to detect.
        let module =
            mk_module_with_classified_param_and_context(Maybe::None, "Logger", List::new());
        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("secret");
        let diags = check_safety(&[module], policy);
        let leak_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message().to_string().contains("low-classification sink"))
            .collect();
        assert_eq!(
            leak_diags.len(),
            0,
            "unclassified params must not trigger sink leak"
        );
    }

    #[test]
    fn mls_phase3a_non_sink_context_does_not_trigger() {
        // Pin: classified param + non-sink context (e.g.
        // "Database") = no leak. Database is a classified-data
        // CONSUMER, not a sink.
        let mut func_attrs = List::new();
        func_attrs.push(mk_classification_attr("secret"));
        let module = mk_module_with_classified_param_and_context(
            Maybe::Some("secret"),
            "Database",
            func_attrs,
        );
        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("secret");
        let diags = check_safety(&[module], policy);
        let leak_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message().to_string().contains("low-classification sink"))
            .collect();
        assert_eq!(
            leak_diags.len(),
            0,
            "non-sink contexts must not trigger leak"
        );
    }

    #[test]
    fn mls_phase3a_recognizes_all_default_sinks() {
        // Pin: every entry in DEFAULT_LOW_CLASSIFICATION_SINKS is
        // recognized when used as the final path segment.
        for sink in &[
            "FS",
            "FileSystem",
            "Network",
            "Stdout",
            "Stderr",
            "Tracing",
            "Telemetry",
        ] {
            let mut func_attrs = List::new();
            func_attrs.push(mk_classification_attr("secret"));
            let module = mk_module_with_classified_param_and_context(
                Maybe::Some("secret"),
                sink,
                func_attrs,
            );
            let mut policy = SafetyPolicy::permissive();
            policy.mls_level = verum_common::Text::from("secret");
            let diags = check_safety(&[module], policy);
            let leak_count = diags
                .iter()
                .filter(|d| d.message().to_string().contains("low-classification sink"))
                .count();
            assert_eq!(
                leak_count, 1,
                "sink {:?} must trigger leak diagnostic",
                sink
            );
        }
    }

    #[test]
    fn mls_phase3a_inactive_under_public_floor() {
        // Pin: when manifest mls_level is "public" (the default),
        // Phase 3a is dormant — Logger usage with classified
        // params produces no diagnostic. Phase 3a only activates
        // when the user opts into a non-public floor.
        let mut func_attrs = List::new();
        func_attrs.push(mk_classification_attr("secret"));
        let module = mk_module_with_classified_param_and_context(
            Maybe::Some("secret"),
            "Logger",
            func_attrs,
        );
        let mut policy = SafetyPolicy::permissive();
        policy.mls_level = verum_common::Text::from("public");
        let diags = check_safety(&[module], policy);
        let leak_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message().to_string().contains("low-classification sink"))
            .collect();
        assert_eq!(
            leak_diags.len(),
            0,
            "Phase 3a must be dormant under public floor"
        );
    }
}
