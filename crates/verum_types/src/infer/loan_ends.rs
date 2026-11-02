//! Statement boundaries at which a reference's last scalar copy is established.
//! Other loans retain their lexical lifetime. The holder must be local to this
//! block, and every use must read or write the scalar without passing on its place.
use super::TypeChecker;
use crate::ty::Type;
use std::collections::{HashMap, HashSet};
use verum_ast::{
    Block, Expr, ExprKind, LiteralKind, PatternKind, Stmt, StmtKind, UnOp,
    visitor::{Visitor, walk_expr, walk_stmt},
};
use verum_common::Text;

#[derive(Default)]
pub(super) struct LoanEnds(HashMap<usize, Vec<Text>>);

impl LoanEnds {
    pub(super) fn for_block(block: &Block) -> Self {
        let mut scan = ScalarUses::default();
        // Only direct holders created in this activation can end here.
        // In particular, a use inside a loop body cannot expire an outer loan
        // which the next iteration may read again.
        for stmt in &block.stmts {
            if let StmtKind::Let {
                pattern,
                value: Some(value),
                ..
            } = &stmt.kind
                && let PatternKind::Ident {
                    name,
                    by_ref: false,
                    subpattern: None,
                    ..
                } = &pattern.kind
                && matches!(
                    value.kind,
                    ExprKind::Unary {
                        op: UnOp::Ref | UnOp::RefMut | UnOp::RefChecked | UnOp::RefCheckedMut,
                        ..
                    }
                )
            {
                scan.candidates.insert(name.name.clone());
            }
        }
        if scan.candidates.is_empty() {
            return Self::default();
        }
        for (index, stmt) in block.stmts.iter().enumerate() {
            scan.statement = index;
            scan.visit_stmt(stmt);
            if scan.opaque {
                return Self::default();
            }
        }
        if let Some(tail) = &block.expr {
            scan.statement = block.stmts.len();
            scan.visit_expr(tail);
        }
        let mut ends = Self::default();
        if !scan.opaque {
            for (name, index) in scan.last {
                if scan.candidates.contains(&name) {
                    ends.0.entry(index).or_default().push(name);
                }
            }
        }
        ends
    }

    pub(super) fn complete(&self, index: usize, checker: &mut TypeChecker) {
        let Some(holders) = self.0.get(&index) else {
            return;
        };
        for holder in holders {
            let Some(scheme) = checker.ctx.env.lookup(holder) else {
                continue;
            };
            if matches!(checker.unifier.apply(&scheme.ty),
                Type::Reference { inner, .. } | Type::CheckedReference { inner, .. }
                if matches!(*inner, Type::Int | Type::Float | Type::Bool | Type::Char | Type::Unit))
            {
                checker.borrow_tracker.release_borrow_at_last_use(holder);
            }
        }
    }
}

#[derive(Default)]
struct ScalarUses {
    candidates: HashSet<Text>,
    declarations: HashSet<Text>,
    last: HashMap<Text, usize>,
    statement: usize,
    depth: usize,
    opaque: bool,
}

impl ScalarUses {
    fn copy(&mut self, expr: &Expr) {
        if let ExprKind::Unary {
            op: UnOp::Deref,
            expr: holder,
        } = &expr.kind
            && let ExprKind::Path(path) = &holder.kind
            && let Some(name) = path.as_ident()
        {
            if self.candidates.contains(&name.name) {
                self.last.insert(name.name.clone(), self.statement);
            }
        } else {
            self.visit_expr(expr);
        }
    }
}

impl Visitor for ScalarUses {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if !stmt.attributes.is_empty() {
            self.opaque = true;
            return;
        }
        match &stmt.kind {
            StmtKind::Let { pattern, ty, value } => {
                let PatternKind::Ident {
                    name,
                    by_ref: false,
                    subpattern: None,
                    ..
                } = &pattern.kind
                else {
                    self.opaque = true;
                    return;
                };
                if !self.declarations.insert(name.name.clone()) {
                    self.candidates.remove(&name.name);
                }
                if ty.is_none()
                    && let Some(value) = value
                {
                    self.copy(value);
                } else {
                    walk_stmt(self, stmt);
                }
            }
            StmtKind::Expr { .. } | StmtKind::Empty => walk_stmt(self, stmt),
            // Deferred code and nested declarations can retain references past
            // this statement. No early lifetime end without escape evidence.
            _ => self.opaque = true,
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        if self.opaque || self.depth == 128 {
            self.opaque = true;
            return;
        }
        self.depth += 1;
        match &expr.kind {
            ExprKind::Path(path) => {
                self.candidates.remove(path.last_segment_name());
                if let Some(verum_ast::ty::PathSegment::Name(root)) = path.segments.first() {
                    self.candidates.remove(&root.name);
                }
            }
            ExprKind::Binary { op, left, right }
                if op.is_comparison() && *op != verum_ast::BinOp::In =>
            {
                self.copy(left);
                self.copy(right);
            }
            ExprKind::Binary { op, left, right } if op.is_assignment() => {
                self.copy(left);
                self.visit_expr(right);
            }
            ExprKind::Literal(lit)
                if matches!(
                    lit.kind,
                    LiteralKind::Int(_)
                        | LiteralKind::Float(_)
                        | LiteralKind::Text(_)
                        | LiteralKind::Char(_)
                        | LiteralKind::Bool(_)
                        | LiteralKind::ByteChar(_)
                        | LiteralKind::ByteString(_)
                ) => {}
            ExprKind::Call { type_args, .. } | ExprKind::MethodCall { type_args, .. }
                if type_args.is_empty() =>
            {
                walk_expr(self, expr)
            }
            ExprKind::Binary { .. }
            | ExprKind::Unary { .. }
            | ExprKind::Paren(_)
            | ExprKind::Index { .. }
            | ExprKind::Field { .. }
            | ExprKind::Range { .. }
            | ExprKind::Array(_)
            | ExprKind::Tuple(_)
            | ExprKind::Block(_)
            | ExprKind::For { .. }
            | ExprKind::While { .. }
            | ExprKind::Loop { .. } => walk_expr(self, expr),
            _ => self.opaque = true,
        }
        self.depth -= 1;
    }

    fn visit_type(&mut self, ty: &verum_ast::ty::Type) {
        match &ty.kind {
            verum_ast::ty::TypeKind::Path(path) => {
                self.candidates.remove(path.last_segment_name());
            }
            _ => self.opaque = true,
        }
    }

    fn visit_pattern(&mut self, pat: &verum_ast::Pattern) {
        match &pat.kind {
            PatternKind::Wildcard => {}
            PatternKind::Ident { name, .. } => {
                self.candidates.remove(&name.name);
            }
            _ => self.opaque = true,
        }
    }
}
