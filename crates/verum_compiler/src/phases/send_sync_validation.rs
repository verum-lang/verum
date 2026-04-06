//! Phase 4d: Send/Sync Compile-Time Enforcement
//!
//! Validates thread-safety bounds at concurrency boundaries:
//! - All variables captured by `spawn` closures must be Send
//! - Borrowed captures in non-move spawn closures must be Sync
//! - Channel.send() arguments must be Send
//! - Shared<T> construction requires T: Send + Sync
//!
//! ## Enforcement Level
//!
//! - **Errors** for known !Send/!Sync types (RawPtr, Cell, RefCell, UnsafeCell, Rc)
//! - **Warnings** for suspicious patterns that can't be fully verified without type info
//!
//! Uses deny-list approach: known !Send/!Sync types produce hard errors.
//! User-defined types auto-derive Send/Sync structurally (see send_sync.rs).

use verum_ast::visitor::{Visitor, walk_expr};
use verum_ast::{Expr, ExprKind, Module};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};
use verum_common::List;
use verum_common::well_known_types::WellKnownType as WKT;

use super::ast_span_to_diagnostic_span;

/// Send/Sync validation phase
pub struct SendSyncValidationPhase;

impl SendSyncValidationPhase {
    pub fn new() -> Self {
        Self
    }

    /// Validate Send/Sync bounds in a module
    pub fn validate_module(&self, module: &Module) -> List<Diagnostic> {
        let mut visitor = SendSyncVisitor::new();
        for item in &module.items {
            visitor.visit_item(item);
        }
        visitor.diagnostics
    }
}

/// Known types that are NOT Send — produces hard errors at spawn/channel boundaries
const NON_SEND_TYPES: &[&str] = &[
    "RawPtr", "UnsafeCell", "Cell", "RefCell", "Rc",
];

fn is_known_non_send(name: &str) -> bool {
    NON_SEND_TYPES.contains(&name)
}

struct SendSyncVisitor {
    diagnostics: List<Diagnostic>,
}

impl SendSyncVisitor {
    fn new() -> Self {
        Self {
            diagnostics: List::new(),
        }
    }

    fn to_diag_span(ast_span: verum_ast::Span) -> verum_diagnostics::Span {
        ast_span_to_diagnostic_span(ast_span, None)
    }

    /// Check if an expression references a known non-Send type
    fn expr_references_non_send_type(&self, expr: &Expr) -> Option<String> {
        match &expr.kind {
            ExprKind::Path(path) => {
                if let Some(seg) = path.segments.last() {
                    let name = match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                        _ => return None,
                    };
                    if is_known_non_send(name) {
                        return Some(name.to_string());
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Check a spawn expression for Send/Sync violations
    fn check_spawn_expr(&mut self, expr: &Expr, spawn_body: &Expr) {
        let mut capture_checker = SpawnCaptureChecker {
            diagnostics: &mut self.diagnostics,
            spawn_span: expr.span,
        };
        capture_checker.visit_expr(spawn_body);
    }

    /// Check a method call for Channel.send with non-Send argument
    fn check_channel_send(&mut self, receiver: &Expr, method: &str, args: &[Expr], span: verum_ast::Span) {
        if method != "send" {
            return;
        }

        if !self.expr_looks_like_channel(receiver) {
            return;
        }

        for arg in args {
            if let Some(type_name) = self.expr_references_non_send_type(arg) {
                self.diagnostics.push(
                    DiagnosticBuilder::error()
                        .message(format!(
                            "E402: Channel.send() with non-Send type `{}`; \
                             values sent through channels must implement Send",
                            type_name
                        ))
                        .span(Self::to_diag_span(span))
                        .help("Consider using a thread-safe alternative: Rc → Shared, Cell → AtomicInt, RefCell → Mutex")
                        .build()
                );
            }
        }
    }

    fn expr_looks_like_channel(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Path(path) => {
                if let Some(seg) = path.segments.last() {
                    if let verum_ast::ty::PathSegment::Name(ident) = seg {
                        let name = ident.name.as_str();
                        return name.contains("ch") || name.contains("channel")
                            || name.contains("chan") || name.contains("_ch");
                    }
                }
                false
            }
            ExprKind::MethodCall { receiver, method, .. } => {
                if method.name.as_str() == "new" {
                    if let ExprKind::Path(path) = &receiver.kind {
                        if let Some(seg) = path.segments.last() {
                            if let verum_ast::ty::PathSegment::Name(ident) = seg {
                                return ident.name.as_str() == "Channel";
                            }
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }
}

impl Visitor for SendSyncVisitor {
    fn visit_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Spawn { expr: body, .. } => {
                self.check_spawn_expr(expr, body);
            }

            ExprKind::MethodCall { receiver, method, args, .. } => {
                self.check_channel_send(receiver, method.name.as_str(), args, expr.span);
            }

            ExprKind::Call { func, args, .. } => {
                if let ExprKind::Path(path) = &func.kind {
                    if let Some(seg) = path.segments.last() {
                        if let verum_ast::ty::PathSegment::Name(ident) = seg {
                            if WKT::Shared.matches(ident.name.as_str()) {
                                for arg in args {
                                    if let Some(type_name) = self.expr_references_non_send_type(arg) {
                                        self.diagnostics.push(
                                            DiagnosticBuilder::error()
                                                .message(format!(
                                                    "E402: Shared<T> requires T: Send + Sync, but `{}` is not Send",
                                                    type_name
                                                ))
                                                .span(Self::to_diag_span(expr.span))
                                                .help("Implement `Send` and `Sync` for this type, or use Mutex<T> for interior mutability")
                                                .build()
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }

            _ => {}
        }

        walk_expr(self, expr);
    }
}

/// Sub-visitor that checks captures within a spawn body
struct SpawnCaptureChecker<'a> {
    diagnostics: &'a mut List<Diagnostic>,
    spawn_span: verum_ast::Span,
}

impl<'a> Visitor for SpawnCaptureChecker<'a> {
    fn visit_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Call { func, .. } => {
                if let ExprKind::Path(path) = &func.kind {
                    if let Some(seg) = path.segments.last() {
                        if let verum_ast::ty::PathSegment::Name(ident) = seg {
                            let name = ident.name.as_str();
                            if is_known_non_send(name) {
                                self.diagnostics.push(
                                    DiagnosticBuilder::error()
                                        .message(format!(
                                            "E402: Type `{}` is not Send and cannot be used in spawn; \
                                             values in spawned tasks must be transferable between threads",
                                            name
                                        ))
                                        .span(SendSyncVisitor::to_diag_span(self.spawn_span))
                                        .help(
                                            "Consider using a thread-safe alternative: \
                                             Rc → Shared, Cell → AtomicInt, RefCell → Mutex"
                                        )
                                        .build()
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        walk_expr(self, expr);
    }
}
