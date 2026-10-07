//! Bounded scalar evaluation shared by array counts and layout producers.
//!
//! This is deliberately smaller than the meta interpreter: checked i64 arithmetic,
//! booleans, lazy conditionals and pure lexical blocks. Declaration lookup and
//! layout queries belong to the caller; no arbitrary function is executed here.
//! Blocks support immutable, unannotated identifier bindings and scalar expression
//! statements. Mutation, typed/destructured bindings, loops and if-let refuse.
//! A concrete array count must additionally be nonnegative at the consumer.
use crate::{
    expr::{BinOp, Block, ConditionKind, Expr, ExprKind, UnOp},
    literal::LiteralKind,
    pattern::PatternKind,
    span::Span,
    stmt::StmtKind,
    ty::PathSegment,
};
use verum_common::{List, Text};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scalar {
    Int(i64),
    Bool(bool),
}
impl Scalar {
    pub fn as_i64(self) -> Option<i64> {
        if let Self::Int(value) = self {
            Some(value)
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub enum EvalError<E> {
    InvalidArithmetic(Span),
    InvalidType(Span),
    Limit(Span),
    Unsupported(Span),
    Unresolved(Span),
    Resolver(E),
}

/// Resolve only declaration-owned leaves. `depth` crosses resolver boundaries so
/// a dependency chain cannot restart the recursion budget at every declaration.
pub fn evaluate<E>(
    expr: &Expr,
    depth: usize,
    resolve: &mut impl FnMut(&Expr, usize) -> Result<Option<Scalar>, E>,
) -> Result<Scalar, EvalError<E>> {
    Evaluator {
        resolve,
        locals: List::new(),
        steps: 0,
    }
    .expr(expr, depth)
}

struct Evaluator<'a, F> {
    resolve: &'a mut F,
    locals: List<(Text, Scalar)>,
    steps: usize,
}
impl<E, F: FnMut(&Expr, usize) -> Result<Option<Scalar>, E>> Evaluator<'_, F> {
    fn expr(&mut self, expr: &Expr, depth: usize) -> Result<Scalar, EvalError<E>> {
        self.steps += 1;
        if depth >= 128 || self.steps > 4096 {
            return Err(EvalError::Limit(expr.span));
        }
        let checked = |value: Option<i64>| {
            value
                .map(Scalar::Int)
                .ok_or(EvalError::InvalidArithmetic(expr.span))
        };
        match &expr.kind {
            ExprKind::Literal(literal) => match &literal.kind {
                LiteralKind::Int(value) => checked(i64::try_from(value.value).ok()),
                LiteralKind::Bool(value) => Ok(Scalar::Bool(*value)),
                _ => Err(EvalError::Unsupported(expr.span)),
            },
            ExprKind::Paren(inner) => self.expr(inner, depth + 1),
            ExprKind::Unary {
                op: UnOp::Neg,
                expr: inner,
            } => {
                // The positive magnitude of i64::MIN is not itself an i64.
                if let ExprKind::Literal(literal) = &inner.kind
                    && let LiteralKind::Int(value) = &literal.kind
                {
                    return checked(
                        value
                            .value
                            .checked_neg()
                            .and_then(|value| i64::try_from(value).ok()),
                    );
                }
                checked(self.integer(inner, depth + 1)?.checked_neg())
            }
            ExprKind::Unary {
                op: UnOp::BitNot,
                expr: inner,
            } => Ok(Scalar::Int(!self.integer(inner, depth + 1)?)),
            ExprKind::Unary {
                op: UnOp::Not,
                expr: inner,
            } => Ok(Scalar::Bool(!self.boolean(inner, depth + 1)?)),
            ExprKind::Binary { op, left, right } => {
                if *op == BinOp::And {
                    return Ok(Scalar::Bool(
                        self.boolean(left, depth + 1)? && self.boolean(right, depth + 1)?,
                    ));
                }
                if *op == BinOp::Or {
                    return Ok(Scalar::Bool(
                        self.boolean(left, depth + 1)? || self.boolean(right, depth + 1)?,
                    ));
                }
                let lhs = self.expr(left, depth + 1)?;
                let rhs = self.expr(right, depth + 1)?;
                if matches!(op, BinOp::Eq | BinOp::Ne) {
                    if core::mem::discriminant(&lhs) != core::mem::discriminant(&rhs) {
                        return Err(EvalError::InvalidType(expr.span));
                    }
                    return Ok(Scalar::Bool(if *op == BinOp::Eq {
                        lhs == rhs
                    } else {
                        lhs != rhs
                    }));
                }
                let (Scalar::Int(lhs), Scalar::Int(rhs)) = (lhs, rhs) else {
                    return Err(EvalError::InvalidType(expr.span));
                };
                match op {
                    BinOp::Add => checked(lhs.checked_add(rhs)),
                    BinOp::Sub => checked(lhs.checked_sub(rhs)),
                    BinOp::Mul => checked(lhs.checked_mul(rhs)),
                    BinOp::Div => checked(lhs.checked_div(rhs)),
                    BinOp::Rem => checked(lhs.checked_rem(rhs)),
                    BinOp::BitAnd => Ok(Scalar::Int(lhs & rhs)),
                    BinOp::BitOr => Ok(Scalar::Int(lhs | rhs)),
                    BinOp::BitXor => Ok(Scalar::Int(lhs ^ rhs)),
                    BinOp::Shl => checked(
                        u32::try_from(rhs)
                            .ok()
                            .filter(|shift| *shift < i64::BITS)
                            .and_then(|shift| i64::try_from((lhs as i128) << shift).ok()),
                    ),
                    BinOp::Shr => checked(
                        u32::try_from(rhs)
                            .ok()
                            .and_then(|shift| lhs.checked_shr(shift)),
                    ),
                    BinOp::Lt => Ok(Scalar::Bool(lhs < rhs)),
                    BinOp::Le => Ok(Scalar::Bool(lhs <= rhs)),
                    BinOp::Gt => Ok(Scalar::Bool(lhs > rhs)),
                    BinOp::Ge => Ok(Scalar::Bool(lhs >= rhs)),
                    _ => Err(EvalError::Unsupported(expr.span)),
                }
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let mut selected = true;
                for condition in &condition.conditions {
                    let ConditionKind::Expr(condition) = condition else {
                        return Err(EvalError::Unsupported(expr.span));
                    };
                    if !self.boolean(condition, depth + 1)? {
                        selected = false;
                        break;
                    }
                }
                if selected {
                    self.block(then_branch, depth + 1)
                } else if let Some(branch) = else_branch {
                    self.expr(branch, depth + 1)
                } else {
                    Err(EvalError::InvalidType(expr.span))
                }
            }
            ExprKind::Block(block) => self.block(block, depth + 1),
            ExprKind::Path(path) => {
                if let [PathSegment::Name(name)] = path.segments.as_slice()
                    && let Some((_, value)) = self
                        .locals
                        .iter()
                        .rev()
                        .find(|(local, _)| local.as_str() == name.name.as_str())
                {
                    return Ok(*value);
                }
                if self.projection_mentions_local(expr) {
                    return Err(EvalError::Unsupported(expr.span));
                }
                (self.resolve)(expr, depth + 1)
                    .map_err(EvalError::Resolver)?
                    .ok_or(EvalError::Unresolved(expr.span))
            }
            ExprKind::Field { .. } | ExprKind::TypeProperty { .. } | ExprKind::Call { .. } => {
                // Declaration/layout resolvers do not own this evaluator's
                // lexical locals. Never resolve a local projection as a global
                // type or constant merely because their spellings coincide.
                if self.projection_mentions_local(expr) {
                    return Err(EvalError::Unsupported(expr.span));
                }
                (self.resolve)(expr, depth + 1)
                    .map_err(EvalError::Resolver)?
                    .ok_or(EvalError::Unresolved(expr.span))
            }
            _ => Err(EvalError::Unsupported(expr.span)),
        }
    }

    fn projection_mentions_local(&self, expr: &Expr) -> bool {
        use crate::visitor::{Visitor, walk_expr, walk_type};
        struct LocalUse<'a> {
            locals: &'a [(Text, Scalar)],
            found: bool,
            depth: usize,
            steps: usize,
        }
        impl Visitor for LocalUse<'_> {
            fn visit_expr(&mut self, expr: &Expr) {
                self.steps += 1;
                if self.depth >= 128 || self.steps > 4096 {
                    self.found = true;
                }
                if self.found {
                    return;
                }
                self.depth += 1;
                walk_expr(self, expr);
                self.depth -= 1;
            }
            fn visit_type(&mut self, ty: &crate::ty::Type) {
                self.steps += 1;
                if self.depth >= 128 || self.steps > 4096 {
                    self.found = true;
                }
                if self.found {
                    return;
                }
                self.depth += 1;
                walk_type(self, ty);
                self.depth -= 1;
            }
            fn visit_path(&mut self, path: &crate::ty::Path) {
                if let Some(PathSegment::Name(name)) = path.segments.first() {
                    self.found |= self.locals.iter().any(|(local, _)| local == &name.name);
                }
            }
        }
        if self.locals.is_empty() {
            return false;
        }
        let mut visitor = LocalUse {
            locals: self.locals.as_slice(),
            found: false,
            depth: 0,
            steps: 0,
        };
        visitor.visit_expr(expr);
        visitor.found
    }

    fn integer(&mut self, expr: &Expr, depth: usize) -> Result<i64, EvalError<E>> {
        self.expr(expr, depth)?
            .as_i64()
            .ok_or(EvalError::InvalidType(expr.span))
    }
    fn boolean(&mut self, expr: &Expr, depth: usize) -> Result<bool, EvalError<E>> {
        match self.expr(expr, depth)? {
            Scalar::Bool(value) => Ok(value),
            _ => Err(EvalError::InvalidType(expr.span)),
        }
    }
    fn block(&mut self, block: &Block, depth: usize) -> Result<Scalar, EvalError<E>> {
        if depth >= 128 {
            return Err(EvalError::Limit(block.span));
        }
        let mark = self.locals.len();
        let result = (|| {
            for stmt in &block.stmts {
                match &stmt.kind {
                    StmtKind::Let {
                        pattern,
                        ty: None,
                        value: Some(value),
                    } => {
                        let PatternKind::Ident {
                            name,
                            by_ref: false,
                            mutable: false,
                            subpattern: None,
                        } = &pattern.kind
                        else {
                            return Err(EvalError::Unsupported(stmt.span));
                        };
                        let value = self.expr(value, depth + 1)?;
                        self.locals.push((name.name.clone(), value));
                    }
                    StmtKind::Expr { expr, .. } => {
                        self.expr(expr, depth + 1)?;
                    }
                    StmtKind::Empty => (),
                    _ => return Err(EvalError::Unsupported(stmt.span)),
                }
            }
            match &block.expr {
                Some(expr) => self.expr(expr, depth + 1),
                None => Err(EvalError::InvalidType(block.span)),
            }
        })();
        self.locals.truncate(mark);
        result
    }
}
