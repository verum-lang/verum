//! In-place traversal of the direct sub-expressions of an expression.
//!
//! # Why this exists
//!
//! [`crate::visitor`] already knows the shape of every `ExprKind` — its
//! [`walk_expr`](crate::visitor::walk_expr) covers all 73 variants with no
//! wildcard arm, so a variant added to the enum cannot slip past it.  That
//! authority is `&Expr` only, and a *rewriter* needs `&mut`.  Rust cannot
//! abstract a closure over mutability, so the mutable side is necessarily a
//! second listing — the same split rustc itself carries between `ast::visit`
//! and `ast::mut_visit`.
//!
//! What must NOT happen is a third, fourth and fifth listing, one per
//! rewriter, each independently incomplete.  That is what happened here, and
//! it cost real soundness:
//!
//! * refinement substitution reached 23 of 73 variants, so `it` was never
//!   substituted inside `(it, 0).0`, `f(n: it)`, `it |> g`, `it?`, `a ?? it`;
//!   the predicate then mentioned an unbound `it`, the solver answered
//!   `unknown`, and an unknown verdict used to be accepted in silence;
//! * free-variable collection reached 6, and it is what capture avoidance
//!   consults;
//! * both termination walks stopped at `TupleIndex`, so `@total` accepted a
//!   function whose only recursive call sat behind a tuple projection.
//!
//! Each was found separately, months apart, by the symptom rather than the
//! cause.  This module is the shared listing so the next rewriter inherits
//! completeness instead of rediscovering the gaps.
//!
//! # What counts as a child
//!
//! Every `Expr` reachable without passing through another `Expr`.  Blocks,
//! match arms, comprehension clauses, recover bodies and `asm!` operands are
//! not expressions, so traversal goes *through* them — a `Match`'s children
//! are the scrutinee plus each arm's guard, body and `with` list, not the
//! arms.
//!
//! Three boundaries are deliberate, and each is a decision rather than an
//! omission:
//!
//! * **Types are not descended into.**  A `Type` can carry expressions — a
//!   nested refinement, a const-generic argument — but a nested refinement
//!   binds its own `it`, so a rewriter that walked in blindly would capture.
//!   Callers that need types have `walk_type`.
//! * **Nested items are not descended into.**  A `fn` inside a block opens a
//!   new scope; substituting an outer variable into it is wrong by
//!   construction.
//! * **Proof justifications and quoted tokens are not descended into.**
//!   `TacticExpr` and `TokenTree` belong to the proof and macro layers, which
//!   have their own traversals.
//!
//! # Binders
//!
//! A blind rewriter must not descend into a node that rebinds the name it is
//! replacing — `Int{ (|it| it)(0) == 0 }` has two different `it`s.  This
//! module does not decide that policy; it reports it through
//! [`introduces_bindings`] so a caller can handle those nodes itself and
//! assert it missed none.

use crate::expr::{
    ArrayExpr, AsmOperandKind, Block, ComprehensionClause, ComprehensionClauseKind, ConditionKind,
    Expr, ExprKind, RecoverBody, StreamLiteralKind,
};
use crate::pattern::MatchArm;
use crate::stmt::StmtKind;

/// Applies `f` to every direct sub-expression of `expr`, in evaluation order.
///
/// `f` is *not* applied to `expr` itself, and this function does not recurse:
/// a caller that wants the whole subtree calls it again from inside `f`.  That
/// keeps the recursion — and any scope bookkeeping around it — with the caller
/// that understands its own binders.
pub fn each_child_expr_mut(expr: &mut Expr, f: &mut dyn FnMut(&mut Expr)) {
    match &mut expr.kind {
        // ---- leaves -------------------------------------------------------
        ExprKind::Literal(_)
        | ExprKind::Path(_)
        | ExprKind::Continue { .. }
        | ExprKind::Inject { .. }
        | ExprKind::TypeBound { .. }
        | ExprKind::TypeProperty { .. }
        | ExprKind::TypeExpr(_)
        | ExprKind::Quote { .. }
        | ExprKind::MacroCall { .. } => {}

        // ---- one child ----------------------------------------------------
        ExprKind::Unary { expr: inner, .. }
        | ExprKind::NamedArg { value: inner, .. }
        | ExprKind::Field { expr: inner, .. }
        | ExprKind::OptionalChain { expr: inner, .. }
        | ExprKind::TupleIndex { expr: inner, .. }
        | ExprKind::Cast { expr: inner, .. }
        | ExprKind::Try(inner)
        | ExprKind::TryBlock(inner)
        | ExprKind::Throw(inner)
        | ExprKind::Yield(inner)
        | ExprKind::Typeof(inner)
        | ExprKind::Await(inner)
        | ExprKind::Spawn { expr: inner, .. }
        | ExprKind::StageEscape { expr: inner, .. }
        | ExprKind::Lift { expr: inner, .. }
        | ExprKind::Paren(inner)
        | ExprKind::DestructuringAssign { value: inner, .. }
        | ExprKind::Is { expr: inner, .. }
        | ExprKind::Attenuate { context: inner, .. }
        | ExprKind::TensorLiteral { data: inner, .. } => f(inner),

        // ---- two children -------------------------------------------------
        ExprKind::Binary { left, right, .. }
        | ExprKind::Pipeline { left, right }
        | ExprKind::NullCoalesce { left, right } => {
            f(left);
            f(right);
        }
        ExprKind::Index { expr: inner, index } => {
            f(inner);
            f(index);
        }
        ExprKind::TryFinally {
            try_block,
            finally_block,
        } => {
            f(try_block);
            f(finally_block);
        }
        ExprKind::UseContext { handler, body, .. } => {
            f(handler);
            f(body);
        }

        // ---- callee plus arguments ----------------------------------------
        ExprKind::Call { func, args, .. } => {
            f(func);
            for arg in args.iter_mut() {
                f(arg);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            f(receiver);
            for arg in args.iter_mut() {
                f(arg);
            }
        }
        ExprKind::MetaFunction { args, .. } => {
            for arg in args.iter_mut() {
                f(arg);
            }
        }

        // ---- flat sequences ------------------------------------------------
        ExprKind::Tuple(items)
        | ExprKind::SetLiteral { elements: items }
        | ExprKind::InterpolatedString { exprs: items, .. } => {
            for item in items.iter_mut() {
                f(item);
            }
        }
        ExprKind::MapLiteral { entries } => {
            for (key, value) in entries.iter_mut() {
                f(key);
                f(value);
            }
        }
        ExprKind::Array(array) => match array {
            ArrayExpr::List(items) => {
                for item in items.iter_mut() {
                    f(item);
                }
            }
            ArrayExpr::Repeat { value, count } => {
                f(value);
                f(count);
            }
        },
        ExprKind::StreamLiteral(stream) => match &mut stream.kind {
            StreamLiteralKind::Elements { elements, .. } => {
                for element in elements.iter_mut() {
                    f(element);
                }
            }
            StreamLiteralKind::Range { start, end, .. } => {
                f(start);
                if let Some(end) = end {
                    f(end);
                }
            }
        },
        ExprKind::Record { fields, base, .. } => {
            for field in fields.iter_mut() {
                if let Some(value) = &mut field.value {
                    f(value);
                }
            }
            if let Some(base) = base {
                f(base);
            }
        }

        // ---- optional children --------------------------------------------
        ExprKind::Break { value, .. } => {
            if let Some(value) = value {
                f(value);
            }
        }
        ExprKind::Return(value) => {
            if let Some(value) = value {
                f(value);
            }
        }
        ExprKind::Range { start, end, .. } => {
            if let Some(start) = start {
                f(start);
            }
            if let Some(end) = end {
                f(end);
            }
        }

        // ---- blocks --------------------------------------------------------
        ExprKind::Block(block)
        | ExprKind::Async(block)
        | ExprKind::Unsafe(block)
        | ExprKind::Meta(block) => each_child_in_block(block, f),

        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            for cond in condition.conditions.iter_mut() {
                match cond {
                    ConditionKind::Expr(inner) => f(inner),
                    ConditionKind::Let { value, .. } => f(value),
                }
            }
            each_child_in_block(then_branch, f);
            if let Some(else_branch) = else_branch {
                f(else_branch);
            }
        }

        // ---- loops ---------------------------------------------------------
        ExprKind::Loop {
            body, invariants, ..
        } => {
            each_child_in_block(body, f);
            for invariant in invariants.iter_mut() {
                f(invariant);
            }
        }
        ExprKind::While {
            condition,
            body,
            invariants,
            decreases,
            ..
        } => {
            f(condition);
            each_child_in_block(body, f);
            for clause in invariants.iter_mut().chain(decreases.iter_mut()) {
                f(clause);
            }
        }
        ExprKind::For {
            iter,
            body,
            invariants,
            decreases,
            ..
        } => {
            f(iter);
            each_child_in_block(body, f);
            for clause in invariants.iter_mut().chain(decreases.iter_mut()) {
                f(clause);
            }
        }
        ExprKind::ForAwait {
            async_iterable,
            body,
            invariants,
            decreases,
            ..
        } => {
            f(async_iterable);
            each_child_in_block(body, f);
            for clause in invariants.iter_mut().chain(decreases.iter_mut()) {
                f(clause);
            }
        }

        // ---- arms ----------------------------------------------------------
        ExprKind::Match { expr: scrutinee, arms } => {
            f(scrutinee);
            each_child_in_arms(arms, f);
        }
        ExprKind::TryRecover { try_block, recover } => {
            f(try_block);
            each_child_in_recover(recover, f);
        }
        ExprKind::TryRecoverFinally {
            try_block,
            recover,
            finally_block,
        } => {
            f(try_block);
            each_child_in_recover(recover, f);
            f(finally_block);
        }
        ExprKind::Select { arms, .. } => {
            for arm in arms.iter_mut() {
                if let Some(future) = &mut arm.future {
                    f(future);
                }
                if let Some(guard) = &mut arm.guard {
                    f(guard);
                }
                f(&mut arm.body);
            }
        }
        ExprKind::Nursery {
            options,
            body,
            on_cancel,
            recover,
            ..
        } => {
            if let Some(timeout) = &mut options.timeout {
                f(timeout);
            }
            if let Some(max_tasks) = &mut options.max_tasks {
                f(max_tasks);
            }
            each_child_in_block(body, f);
            if let Some(on_cancel) = on_cancel {
                each_child_in_block(on_cancel, f);
            }
            if let Some(recover) = recover {
                each_child_in_recover(recover, f);
            }
        }
        ExprKind::CopatternBody { arms, .. } => {
            for arm in arms.iter_mut() {
                f(&mut arm.body);
            }
        }

        // ---- comprehensions -------------------------------------------------
        ExprKind::Comprehension { expr: inner, clauses }
        | ExprKind::StreamComprehension { expr: inner, clauses }
        | ExprKind::SetComprehension { expr: inner, clauses }
        | ExprKind::GeneratorComprehension { expr: inner, clauses } => {
            each_child_in_clauses(clauses, f);
            f(inner);
        }
        ExprKind::MapComprehension {
            key_expr,
            value_expr,
            clauses,
        } => {
            each_child_in_clauses(clauses, f);
            f(key_expr);
            f(value_expr);
        }

        // ---- quantifiers ----------------------------------------------------
        ExprKind::Forall { bindings, body } | ExprKind::Exists { bindings, body } => {
            for binding in bindings.iter_mut() {
                if let Some(domain) = &mut binding.domain {
                    f(domain);
                }
                if let Some(guard) = &mut binding.guard {
                    f(guard);
                }
            }
            f(body);
        }

        // ---- closures --------------------------------------------------------
        ExprKind::Closure { body, .. } => f(body),

        // ---- inline assembly -------------------------------------------------
        ExprKind::InlineAsm { operands, .. } => {
            for operand in operands.iter_mut() {
                match &mut operand.kind {
                    AsmOperandKind::In { expr: inner, .. }
                    | AsmOperandKind::Out { place: inner, .. }
                    | AsmOperandKind::InOut { place: inner, .. }
                    | AsmOperandKind::Const { expr: inner } => f(inner),
                    AsmOperandKind::InLateOut {
                        in_expr, out_place, ..
                    } => {
                        f(in_expr);
                        f(out_place);
                    }
                    AsmOperandKind::Sym { .. } | AsmOperandKind::Clobber { .. } => {}
                }
            }
        }

        // ---- calculation chains -----------------------------------------------
        ExprKind::CalcBlock(chain) => {
            f(&mut chain.start);
            for step in chain.steps.iter_mut() {
                f(&mut step.target);
            }
        }
    }
}

/// Applies `f` to every expression a block holds directly.
///
/// Nested items are skipped: a `fn` declared inside a block opens its own
/// scope, so a rewriter replacing an outer name must not reach into it.
fn each_child_in_block(block: &mut Block, f: &mut dyn FnMut(&mut Expr)) {
    for stmt in block.stmts.iter_mut() {
        match &mut stmt.kind {
            StmtKind::Let { value, .. } => {
                if let Some(value) = value {
                    f(value);
                }
            }
            StmtKind::LetElse {
                value, else_block, ..
            } => {
                f(value);
                each_child_in_block(else_block, f);
            }
            StmtKind::Expr { expr, .. } => f(expr),
            StmtKind::Defer(expr) | StmtKind::Errdefer(expr) => f(expr),
            StmtKind::Provide { value, .. } => f(value),
            StmtKind::ProvideScope { value, block, .. } => {
                f(value);
                f(block);
            }
            StmtKind::Item(_) | StmtKind::Empty => {}
        }
    }
    if let Some(tail) = &mut block.expr {
        f(tail);
    }
}

fn each_child_in_arms(arms: &mut [MatchArm], f: &mut dyn FnMut(&mut Expr)) {
    for arm in arms.iter_mut() {
        if let Some(guard) = &mut arm.guard {
            f(guard);
        }
        if let Some(with_clause) = &mut arm.with_clause {
            for expr in with_clause.iter_mut() {
                f(expr);
            }
        }
        f(&mut arm.body);
    }
}

fn each_child_in_recover(recover: &mut RecoverBody, f: &mut dyn FnMut(&mut Expr)) {
    match recover {
        RecoverBody::MatchArms { arms, .. } => each_child_in_arms(arms, f),
        RecoverBody::Closure { body, .. } => f(body),
    }
}

fn each_child_in_clauses(clauses: &mut [ComprehensionClause], f: &mut dyn FnMut(&mut Expr)) {
    for clause in clauses.iter_mut() {
        match &mut clause.kind {
            ComprehensionClauseKind::For { iter, .. } => f(iter),
            ComprehensionClauseKind::If(cond) => f(cond),
            ComprehensionClauseKind::Let { value, .. } => f(value),
        }
    }
}

/// Whether this node makes new bindings visible to some of its children.
///
/// A rewriter that replaces a *name* must handle these itself — descending
/// blindly into a node that rebinds that name captures it.  A caller ends its
/// explicit arms by ASKING this question and refusing when the answer is yes,
/// before falling through to [`each_child_expr_mut`]; a binder added to the
/// enum later is then refused rather than silently captured.
///
/// It was a `debug_assert!` here once, and that is not a guard: release
/// builds delete it, so the capture it described happened anyway and said
/// nothing.  The check has to run where the code runs.
///
/// `Is` and `DestructuringAssign` are deliberately absent.  Both carry a
/// pattern, but neither scopes it over a child: `x is Some(n)` publishes `n`
/// to the statements that follow, and a destructuring assignment writes to
/// places that already exist.
pub fn introduces_bindings(kind: &ExprKind) -> bool {
    binder_name(kind).is_some()
}

/// The name of the binding form, when this node is one.
///
/// One list, two questions.  A rewriter that has to REFUSE a binder it has
/// no scope rule for should be able to say WHICH one it refused — a refusal
/// that names nothing is a dead end for whoever reads the log, and the
/// alternative (each caller keeping its own copy of the list to render a
/// name) is how the list drifts.
pub fn binder_name(kind: &ExprKind) -> Option<&'static str> {
    Some(match kind {
        ExprKind::Closure { .. } => "closure",
        ExprKind::For { .. } => "for",
        ExprKind::ForAwait { .. } => "for await",
        ExprKind::Match { .. } => "match",
        ExprKind::Forall { .. } => "forall",
        ExprKind::Exists { .. } => "exists",
        ExprKind::Select { .. } => "select",
        ExprKind::Comprehension { .. } => "list comprehension",
        ExprKind::StreamComprehension { .. } => "stream comprehension",
        ExprKind::SetComprehension { .. } => "set comprehension",
        ExprKind::GeneratorComprehension { .. } => "generator comprehension",
        ExprKind::MapComprehension { .. } => "map comprehension",
        ExprKind::TryRecover { .. } => "try/recover",
        ExprKind::TryRecoverFinally { .. } => "try/recover/finally",
        ExprKind::Nursery { .. } => "nursery",
        ExprKind::If { .. } => "if",
        ExprKind::Block(_) => "block",
        ExprKind::Async(_) => "async block",
        ExprKind::Unsafe(_) => "unsafe block",
        ExprKind::Meta(_) => "meta block",
        _ => return None,
    })
}
