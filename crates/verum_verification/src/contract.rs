//! Contract Literals System for Verum Verification
//!
//! This module implements the contract#"..." compiler intrinsic for Hoare-logic
//! style contracts embedded directly in the language.
//!
//! # Contract DSL Syntax
//!
//! The contract DSL supports:
//! - `requires` clauses (preconditions)
//! - `ensures` clauses (postconditions)
//! - `invariant` clauses (loop invariants)
//! - `old(expr)` for capturing pre-state values
//! - `result` keyword for return values
//! - `forall` and `exists` quantifiers
//!
//! # Example
//!
//! ```verum
//! @verify(proof)
//! fn transfer_funds(from: &mut Account, to: &mut Account, amount: Money) {
//!     contract#"
//!         requires from.balance >= amount;
//!         ensures from.balance == old(from.balance) - amount;
//!         ensures to.balance == old(to.balance) + amount;
//!     "
//!     // ... implementation
//! }
//! ```
//!
//! # Architecture
//!
//! The contract system consists of:
//! 1. **ContractParser** - Parses contract#"..." literals into ContractSpec AST
//! 2. **ContractSpec** - Structured representation of preconditions, postconditions, invariants
//! 3. **SMT Translation** - Converts contracts to SMT-LIB for verification
//! 4. **Runtime Instrumentation** - Generates runtime assertion code for @verify(runtime)
//!
//! The contract system is a compiler intrinsic (NOT a user-defined tagged literal).
//! It integrates deeply with the type system, SMT solver, and verification modes.
//! The 4-phase pipeline is: (1) parse contract literal into ContractSpec,
//! (2) lower function body to SSA and generate verification conditions in SMT-LIB 2.0,
//! (3) invoke Z3/CVC5 to prove correctness (UNSAT = verified, SAT = counterexample),
//! (4) emit optimized code (verified), compilation error (failed), or runtime checks (incomplete).

use crate::vcgen::{
    Formula, SmtExpr, SourceLocation, VCKind, VarType, Variable, VerificationCondition,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use thiserror::Error;
use verum_ast::span::Span;
use verum_common::{List, Map, Text};

// =============================================================================
// Contract AST Types
// =============================================================================

/// A complete contract specification parsed from a contract# literal.
///
/// Parsed from contract#"..." literals. Contains preconditions (caller must ensure
/// at function entry), postconditions (function guarantees at exit), and invariants
/// (hold at all program points). Supports `old(expr)` for pre-state values,
/// `result` for return values, and `forall`/`exists` quantifiers.
///
/// Example:
/// ```verum
/// contract#"
///     requires x > 0;
///     requires y > 0;
///     ensures result == x + y;
///     ensures result > x && result > y;
/// "
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContractSpec {
    /// Preconditions (caller must ensure these hold at function entry)
    pub preconditions: List<Predicate>,
    /// Postconditions (function guarantees these hold at function exit)
    pub postconditions: List<Predicate>,
    /// Invariants (must hold throughout execution)
    pub invariants: List<Predicate>,
    /// Source span for error reporting
    pub span: Span,
}

impl ContractSpec {
    /// Create a new empty contract specification.
    pub fn new(span: Span) -> Self {
        Self {
            preconditions: List::new(),
            postconditions: List::new(),
            invariants: List::new(),
            span,
        }
    }

    /// Check if the contract is empty.
    pub fn is_empty(&self) -> bool {
        self.preconditions.is_empty()
            && self.postconditions.is_empty()
            && self.invariants.is_empty()
    }

    /// Get all predicates as a flat list.
    pub fn all_predicates(&self) -> List<&Predicate> {
        let mut preds = List::new();
        for p in self.preconditions.iter() {
            preds.push(p);
        }
        for p in self.postconditions.iter() {
            preds.push(p);
        }
        for p in self.invariants.iter() {
            preds.push(p);
        }
        preds
    }

    /// Validate semantic correctness of the contract.
    ///
    /// Checks:
    /// - `result` only appears in postconditions
    /// - `old(expr)` only appears in postconditions
    /// - No circular dependencies
    pub fn validate(&self) -> Result<(), ContractError> {
        // Check preconditions don't use result or old()
        for (idx, pred) in self.preconditions.iter().enumerate() {
            if pred.contains_result() {
                return Err(ContractError::InvalidUsage {
                    message: format!("precondition #{} cannot reference 'result'", idx + 1),
                    span: self.span,
                });
            }
            if pred.contains_old() {
                return Err(ContractError::InvalidUsage {
                    message: format!("precondition #{} cannot use 'old()'", idx + 1),
                    span: self.span,
                });
            }
        }

        // Check invariants don't use result or old()
        for (idx, pred) in self.invariants.iter().enumerate() {
            if pred.contains_result() {
                return Err(ContractError::InvalidUsage {
                    message: format!("invariant #{} cannot reference 'result'", idx + 1),
                    span: self.span,
                });
            }
            if pred.contains_old() {
                return Err(ContractError::InvalidUsage {
                    message: format!("invariant #{} cannot use 'old()'", idx + 1),
                    span: self.span,
                });
            }
        }

        Ok(())
    }

    /// Collect all `old()` expressions that need to be stored before function execution.
    pub fn collect_old_expressions(&self) -> List<OldExpr> {
        let mut old_exprs = List::new();
        for pred in self.postconditions.iter() {
            pred.collect_old_exprs(&mut old_exprs);
        }
        old_exprs
    }

    /// Merge another contract specification into this one.
    pub fn merge(&mut self, other: &ContractSpec) {
        for p in other.preconditions.iter() {
            self.preconditions.push(p.clone());
        }
        for p in other.postconditions.iter() {
            self.postconditions.push(p.clone());
        }
        for i in other.invariants.iter() {
            self.invariants.push(i.clone());
        }
    }
}

impl fmt::Display for ContractSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for pred in self.preconditions.iter() {
            writeln!(f, "requires {};", pred)?;
        }
        for pred in self.postconditions.iter() {
            writeln!(f, "ensures {};", pred)?;
        }
        for pred in self.invariants.iter() {
            writeln!(f, "invariant {};", pred)?;
        }
        Ok(())
    }
}

/// A contract clause (requires/ensures/invariant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContractClause {
    /// Precondition (requires)
    Requires(()),
    /// Postcondition (ensures)
    Ensures(()),
    /// Invariant (invariant)
    Invariant(()),
}

/// A single predicate in a contract clause.
///
/// Predicates are logical expressions that must evaluate to boolean.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Predicate {
    /// The predicate expression
    pub expr: ContractExpr,
    /// Optional label for error messages
    pub label: Option<Text>,
    /// Source span
    pub span: Span,
}

impl Predicate {
    /// Create a new predicate.
    pub fn new(expr: ContractExpr, span: Span) -> Self {
        Self {
            expr,
            label: None,
            span,
        }
    }

    /// Create a predicate with a label.
    pub fn with_label(expr: ContractExpr, label: Text, span: Span) -> Self {
        Self {
            expr,
            label: Some(label),
            span,
        }
    }

    /// Check if this predicate contains a reference to 'result'.
    pub fn contains_result(&self) -> bool {
        self.expr.contains_result()
    }

    /// Check if this predicate contains an 'old()' expression.
    pub fn contains_old(&self) -> bool {
        self.expr.contains_old()
    }

    /// Collect all old() expressions in this predicate.
    pub fn collect_old_exprs(&self, out: &mut List<OldExpr>) {
        self.expr.collect_old_exprs(out);
    }

    /// Convert to verification formula.
    pub fn to_formula(&self) -> Formula {
        self.expr.to_formula()
    }

    /// Convert to SMT expression.
    pub fn to_smt_expr(&self) -> SmtExpr {
        self.expr.to_smt_expr()
    }
}

impl fmt::Display for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.expr)
    }
}

/// Expression nodes for the contract DSL.
///
/// Contract DSL expression nodes. Supports the full subset of expressions valid
/// in contracts: comparison operators, logical connectives, arithmetic, quantifiers
/// (forall/exists), old() value capture, and the result keyword. Provides:
///
/// - **Type safety**: Distinct types for contract-specific constructs like `result` and `old()`
/// - **SMT translation**: Direct conversion to SMT-LIB formulas via `to_formula()` and `to_smt_expr()`
/// - **Scope tracking**: Free variable collection and bound variable tracking for quantifiers
/// - **Validation**: Methods to check context-validity (e.g., `result` only in postconditions)
///
/// ## Supported Contract DSL Keywords
///
/// | Keyword | Meaning | Variant |
/// |---------|---------|---------|
/// | `result` | Return value | `ContractExpr::Result` |
/// | `old(expr)` | Pre-state value | `ContractExpr::Old(OldExpr)` |
/// | `forall x. P` | Universal quantifier | `ContractExpr::Forall` |
/// | `exists x. P` | Existential quantifier | `ContractExpr::Exists` |
/// | `let x = e in body` | Let binding | `ContractExpr::Let` |
///
/// ## Conversion from verum_ast::Expr
///
/// Use `ContractExpr::from_ast()` to convert from the general AST expression type.
/// This conversion validates that the expression is valid in a contract context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ContractExpr {
    /// Boolean true
    True,
    /// Boolean false
    False,
    /// Integer literal
    Int(i64),
    /// Float literal
    Float(f64),
    /// Variable reference
    Var(Text),
    /// The special 'result' keyword (function return value)
    Result,
    /// old(expr) - pre-state value capture
    Old(OldExpr),
    /// Binary operation
    BinOp(ContractBinOp, Box<ContractExpr>, Box<ContractExpr>),
    /// Unary operation
    UnOp(ContractUnOp, Box<ContractExpr>),
    /// Function call
    Call(Text, List<ContractExpr>),
    /// Field access
    Field(Box<ContractExpr>, Text),
    /// Array/map index
    Index(Box<ContractExpr>, Box<ContractExpr>),
    /// Method call
    MethodCall(Box<ContractExpr>, Text, List<ContractExpr>),
    /// Universal quantifier: forall x. P(x) or forall x in range. P(x)
    Forall(QuantifierBinding, Box<ContractExpr>),
    /// Existential quantifier: exists x. P(x)
    Exists(QuantifierBinding, Box<ContractExpr>),
    /// Let binding: let x = e in body
    Let(Text, Box<ContractExpr>, Box<ContractExpr>),
    /// If-then-else
    IfThenElse(Box<ContractExpr>, Box<ContractExpr>, Box<ContractExpr>),
    /// Parenthesized expression
    Paren(Box<ContractExpr>),
}

impl ContractExpr {
    /// Check if this expression contains a reference to 'result'.
    pub fn contains_result(&self) -> bool {
        match self {
            ContractExpr::Result => true,
            ContractExpr::True
            | ContractExpr::False
            | ContractExpr::Int(_)
            | ContractExpr::Float(_)
            | ContractExpr::Var(_) => false,
            ContractExpr::Old(old) => old.inner.contains_result(),
            ContractExpr::BinOp(_, l, r) => l.contains_result() || r.contains_result(),
            ContractExpr::UnOp(_, e) => e.contains_result(),
            ContractExpr::Call(_, args) => args.iter().any(|a| a.contains_result()),
            ContractExpr::Field(e, _) => e.contains_result(),
            ContractExpr::Index(arr, idx) => arr.contains_result() || idx.contains_result(),
            ContractExpr::MethodCall(recv, _, args) => {
                recv.contains_result() || args.iter().any(|a| a.contains_result())
            }
            ContractExpr::Forall(_, body) | ContractExpr::Exists(_, body) => body.contains_result(),
            ContractExpr::Let(_, val, body) => val.contains_result() || body.contains_result(),
            ContractExpr::IfThenElse(c, t, e) => {
                c.contains_result() || t.contains_result() || e.contains_result()
            }
            ContractExpr::Paren(e) => e.contains_result(),
        }
    }

    /// Check if this expression contains an 'old()' expression.
    pub fn contains_old(&self) -> bool {
        match self {
            ContractExpr::Old(_) => true,
            ContractExpr::True
            | ContractExpr::False
            | ContractExpr::Int(_)
            | ContractExpr::Float(_)
            | ContractExpr::Var(_)
            | ContractExpr::Result => false,
            ContractExpr::BinOp(_, l, r) => l.contains_old() || r.contains_old(),
            ContractExpr::UnOp(_, e) => e.contains_old(),
            ContractExpr::Call(_, args) => args.iter().any(|a| a.contains_old()),
            ContractExpr::Field(e, _) => e.contains_old(),
            ContractExpr::Index(arr, idx) => arr.contains_old() || idx.contains_old(),
            ContractExpr::MethodCall(recv, _, args) => {
                recv.contains_old() || args.iter().any(|a| a.contains_old())
            }
            ContractExpr::Forall(_, body) | ContractExpr::Exists(_, body) => body.contains_old(),
            ContractExpr::Let(_, val, body) => val.contains_old() || body.contains_old(),
            ContractExpr::IfThenElse(c, t, e) => {
                c.contains_old() || t.contains_old() || e.contains_old()
            }
            ContractExpr::Paren(e) => e.contains_old(),
        }
    }

    /// Collect all old() expressions.
    pub fn collect_old_exprs(&self, out: &mut List<OldExpr>) {
        match self {
            ContractExpr::Old(old) => {
                out.push(old.clone());
                old.inner.collect_old_exprs(out);
            }
            ContractExpr::True
            | ContractExpr::False
            | ContractExpr::Int(_)
            | ContractExpr::Float(_)
            | ContractExpr::Var(_)
            | ContractExpr::Result => {}
            ContractExpr::BinOp(_, l, r) => {
                l.collect_old_exprs(out);
                r.collect_old_exprs(out);
            }
            ContractExpr::UnOp(_, e) => e.collect_old_exprs(out),
            ContractExpr::Call(_, args) => {
                for a in args.iter() {
                    a.collect_old_exprs(out);
                }
            }
            ContractExpr::Field(e, _) => e.collect_old_exprs(out),
            ContractExpr::Index(arr, idx) => {
                arr.collect_old_exprs(out);
                idx.collect_old_exprs(out);
            }
            ContractExpr::MethodCall(recv, _, args) => {
                recv.collect_old_exprs(out);
                for a in args.iter() {
                    a.collect_old_exprs(out);
                }
            }
            ContractExpr::Forall(_, body) | ContractExpr::Exists(_, body) => {
                body.collect_old_exprs(out);
            }
            ContractExpr::Let(_, val, body) => {
                val.collect_old_exprs(out);
                body.collect_old_exprs(out);
            }
            ContractExpr::IfThenElse(c, t, e) => {
                c.collect_old_exprs(out);
                t.collect_old_exprs(out);
                e.collect_old_exprs(out);
            }
            ContractExpr::Paren(e) => e.collect_old_exprs(out),
        }
    }

    /// Collect free variables.
    pub fn free_variables(&self) -> HashSet<Text> {
        let mut vars = HashSet::new();
        self.collect_free_vars(&mut vars, &HashSet::new());
        vars
    }

    fn collect_free_vars(&self, vars: &mut HashSet<Text>, bound: &HashSet<Text>) {
        match self {
            ContractExpr::Var(name) if !bound.contains(name) => {
                vars.insert(name.clone());
            }
            ContractExpr::Var(_)
            | ContractExpr::True
            | ContractExpr::False
            | ContractExpr::Int(_)
            | ContractExpr::Float(_)
            | ContractExpr::Result => {}
            ContractExpr::Old(old) => old.inner.collect_free_vars(vars, bound),
            ContractExpr::BinOp(_, l, r) => {
                l.collect_free_vars(vars, bound);
                r.collect_free_vars(vars, bound);
            }
            ContractExpr::UnOp(_, e) => e.collect_free_vars(vars, bound),
            ContractExpr::Call(_, args) => {
                for a in args.iter() {
                    a.collect_free_vars(vars, bound);
                }
            }
            ContractExpr::Field(e, _) => e.collect_free_vars(vars, bound),
            ContractExpr::Index(arr, idx) => {
                arr.collect_free_vars(vars, bound);
                idx.collect_free_vars(vars, bound);
            }
            ContractExpr::MethodCall(recv, _, args) => {
                recv.collect_free_vars(vars, bound);
                for a in args.iter() {
                    a.collect_free_vars(vars, bound);
                }
            }
            ContractExpr::Forall(binding, body) | ContractExpr::Exists(binding, body) => {
                let mut new_bound = bound.clone();
                new_bound.insert(binding.variable.clone());
                if let Some(ref range) = binding.range {
                    range.lower.collect_free_vars(vars, bound);
                    range.upper.collect_free_vars(vars, bound);
                }
                body.collect_free_vars(vars, &new_bound);
            }
            ContractExpr::Let(name, val, body) => {
                val.collect_free_vars(vars, bound);
                let mut new_bound = bound.clone();
                new_bound.insert(name.clone());
                body.collect_free_vars(vars, &new_bound);
            }
            ContractExpr::IfThenElse(c, t, e) => {
                c.collect_free_vars(vars, bound);
                t.collect_free_vars(vars, bound);
                e.collect_free_vars(vars, bound);
            }
            ContractExpr::Paren(e) => e.collect_free_vars(vars, bound),
        }
    }

    /// Convert to verification formula.
    pub fn to_formula(&self) -> Formula {
        match self {
            ContractExpr::True => Formula::True,
            ContractExpr::False => Formula::False,
            ContractExpr::Var(name) => Formula::Var(Variable::new(name.clone())),
            ContractExpr::Result => Formula::Var(Variable::result()),
            ContractExpr::BinOp(op, l, r) => {
                let left = l.to_smt_expr();
                let right = r.to_smt_expr();
                match op {
                    ContractBinOp::And => Formula::and([l.to_formula(), r.to_formula()]),
                    ContractBinOp::Or => Formula::or([l.to_formula(), r.to_formula()]),
                    ContractBinOp::Imply => Formula::implies(l.to_formula(), r.to_formula()),
                    ContractBinOp::Eq => Formula::eq(left, right),
                    ContractBinOp::Ne => Formula::Ne(Box::new(left), Box::new(right)),
                    ContractBinOp::Lt => Formula::lt(left, right),
                    ContractBinOp::Le => Formula::le(left, right),
                    ContractBinOp::Gt => Formula::gt(left, right),
                    ContractBinOp::Ge => Formula::ge(left, right),
                    _ => {
                        // Arithmetic operations: wrap in predicate
                        let smt = self.to_smt_expr();
                        Formula::Predicate(Text::from("is_true"), vec![smt].into())
                    }
                }
            }
            ContractExpr::UnOp(ContractUnOp::Not, e) => Formula::not(e.to_formula()),
            ContractExpr::Forall(binding, body) => {
                let var = Variable::typed(binding.variable.clone(), VarType::Int);
                let body_formula = if let Some(ref range) = binding.range {
                    // forall x in [lo, hi). body  =>  forall x. (lo <= x < hi) => body
                    let lo = range.lower.to_smt_expr();
                    let hi = range.upper.to_smt_expr();
                    let x = SmtExpr::var(binding.variable.clone());
                    let in_range = Formula::and([Formula::le(lo, x.clone()), Formula::lt(x, hi)]);
                    Formula::implies(in_range, body.to_formula())
                } else {
                    body.to_formula()
                };
                Formula::Forall(vec![var].into(), Box::new(body_formula))
            }
            ContractExpr::Exists(binding, body) => {
                let var = Variable::typed(binding.variable.clone(), VarType::Int);
                let body_formula = if let Some(ref range) = binding.range {
                    // exists x in [lo, hi). body  =>  exists x. (lo <= x < hi) && body
                    let lo = range.lower.to_smt_expr();
                    let hi = range.upper.to_smt_expr();
                    let x = SmtExpr::var(binding.variable.clone());
                    let in_range = Formula::and([Formula::le(lo, x.clone()), Formula::lt(x, hi)]);
                    Formula::and([in_range, body.to_formula()])
                } else {
                    body.to_formula()
                };
                Formula::Exists(vec![var].into(), Box::new(body_formula))
            }
            ContractExpr::Let(name, val, body) => Formula::Let(
                Variable::new(name.clone()),
                Box::new(val.to_smt_expr()),
                Box::new(body.to_formula()),
            ),
            ContractExpr::IfThenElse(c, t, e) => {
                // (c => t) && (!c => e)
                Formula::and([
                    Formula::implies(c.to_formula(), t.to_formula()),
                    Formula::implies(Formula::not(c.to_formula()), e.to_formula()),
                ])
            }
            ContractExpr::Paren(e) => e.to_formula(),
            _ => {
                // For other expressions, wrap in a predicate
                let smt = self.to_smt_expr();
                Formula::Predicate(Text::from("is_true"), vec![smt].into())
            }
        }
    }

    /// Convert to SMT expression.
    pub fn to_smt_expr(&self) -> SmtExpr {
        match self {
            ContractExpr::True => SmtExpr::BoolConst(true),
            ContractExpr::False => SmtExpr::BoolConst(false),
            ContractExpr::Int(n) => SmtExpr::IntConst(*n),
            ContractExpr::Float(f) => SmtExpr::RealConst(*f),
            ContractExpr::Var(name) => SmtExpr::var(name.clone()),
            ContractExpr::Result => SmtExpr::var("result"),
            ContractExpr::Old(old) => {
                // old(x) becomes old_x in SMT
                let inner_name = old.storage_name();
                SmtExpr::var(inner_name)
            }
            ContractExpr::BinOp(op, l, r) => {
                let left = l.to_smt_expr();
                let right = r.to_smt_expr();
                match op {
                    ContractBinOp::Add => SmtExpr::add(left, right),
                    ContractBinOp::Sub => SmtExpr::sub(left, right),
                    ContractBinOp::Mul => SmtExpr::mul(left, right),
                    ContractBinOp::Div => {
                        SmtExpr::BinOp(crate::vcgen::SmtBinOp::Div, Box::new(left), Box::new(right))
                    }
                    ContractBinOp::Mod => {
                        SmtExpr::BinOp(crate::vcgen::SmtBinOp::Mod, Box::new(left), Box::new(right))
                    }
                    ContractBinOp::Eq => SmtExpr::Apply(Text::from("="), vec![left, right].into()),
                    ContractBinOp::Ne => {
                        SmtExpr::Apply(Text::from("distinct"), vec![left, right].into())
                    }
                    ContractBinOp::Lt => SmtExpr::Apply(Text::from("<"), vec![left, right].into()),
                    ContractBinOp::Le => SmtExpr::Apply(Text::from("<="), vec![left, right].into()),
                    ContractBinOp::Gt => SmtExpr::Apply(Text::from(">"), vec![left, right].into()),
                    ContractBinOp::Ge => SmtExpr::Apply(Text::from(">="), vec![left, right].into()),
                    ContractBinOp::And => {
                        SmtExpr::Apply(Text::from("and"), vec![left, right].into())
                    }
                    ContractBinOp::Or => SmtExpr::Apply(Text::from("or"), vec![left, right].into()),
                    ContractBinOp::Imply => {
                        SmtExpr::Apply(Text::from("=>"), vec![left, right].into())
                    }
                }
            }
            ContractExpr::UnOp(op, e) => {
                let inner = e.to_smt_expr();
                match op {
                    ContractUnOp::Not => SmtExpr::Apply(Text::from("not"), vec![inner].into()),
                    ContractUnOp::Neg => SmtExpr::UnOp(crate::vcgen::SmtUnOp::Neg, Box::new(inner)),
                }
            }
            ContractExpr::Call(name, args) => {
                let smt_args: List<SmtExpr> =
                    args.iter().map(|a| a.to_smt_expr()).collect::<List<_>>();
                SmtExpr::Apply(name.clone(), smt_args)
            }
            ContractExpr::Field(e, field) => {
                let base = e.to_smt_expr();
                SmtExpr::Apply(Text::from(format!("field_{}", field)), vec![base].into())
            }
            ContractExpr::Index(arr, idx) => {
                let arr_smt = arr.to_smt_expr();
                let idx_smt = idx.to_smt_expr();
                SmtExpr::Select(Box::new(arr_smt), Box::new(idx_smt))
            }
            ContractExpr::MethodCall(recv, method, args) => {
                let recv_smt = recv.to_smt_expr();
                let mut all_args = vec![recv_smt];
                all_args.extend(args.iter().map(|a| a.to_smt_expr()));
                SmtExpr::Apply(method.clone(), all_args.into())
            }
            ContractExpr::Forall(_, _) | ContractExpr::Exists(_, _) => {
                // Quantifiers should be handled at the formula level
                SmtExpr::BoolConst(true)
            }
            ContractExpr::Let(name, val, body) => SmtExpr::Let(
                Variable::new(name.clone()),
                Box::new(val.to_smt_expr()),
                Box::new(body.to_smt_expr()),
            ),
            ContractExpr::IfThenElse(c, t, e) => SmtExpr::Ite(
                Box::new(c.to_formula()),
                Box::new(t.to_smt_expr()),
                Box::new(e.to_smt_expr()),
            ),
            ContractExpr::Paren(e) => e.to_smt_expr(),
        }
    }
}

impl fmt::Display for ContractExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContractExpr::True => write!(f, "true"),
            ContractExpr::False => write!(f, "false"),
            ContractExpr::Int(n) => write!(f, "{}", n),
            ContractExpr::Float(n) => write!(f, "{}", n),
            ContractExpr::Var(name) => write!(f, "{}", name),
            ContractExpr::Result => write!(f, "result"),
            ContractExpr::Old(old) => write!(f, "{}", old),
            ContractExpr::BinOp(op, l, r) => write!(f, "({} {} {})", l, op, r),
            ContractExpr::UnOp(op, e) => write!(f, "{}{}", op, e),
            ContractExpr::Call(name, args) => {
                write!(f, "{}(", name)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            ContractExpr::Field(e, field) => write!(f, "{}.{}", e, field),
            ContractExpr::Index(arr, idx) => write!(f, "{}[{}]", arr, idx),
            ContractExpr::MethodCall(recv, method, args) => {
                write!(f, "{}.{}(", recv, method)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            ContractExpr::Forall(binding, body) => {
                write!(f, "forall {}. {}", binding, body)
            }
            ContractExpr::Exists(binding, body) => {
                write!(f, "exists {}. {}", binding, body)
            }
            ContractExpr::Let(name, val, body) => {
                write!(f, "let {} = {} in {}", name, val, body)
            }
            ContractExpr::IfThenElse(c, t, e) => {
                write!(f, "if {} then {} else {}", c, t, e)
            }
            ContractExpr::Paren(e) => write!(f, "({})", e),
        }
    }
}

/// Binary operators for contract expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContractBinOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    // Comparison
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    // Logical
    And,
    Or,
    Imply,
}

impl fmt::Display for ContractBinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContractBinOp::Add => write!(f, "+"),
            ContractBinOp::Sub => write!(f, "-"),
            ContractBinOp::Mul => write!(f, "*"),
            ContractBinOp::Div => write!(f, "/"),
            ContractBinOp::Mod => write!(f, "%"),
            ContractBinOp::Eq => write!(f, "=="),
            ContractBinOp::Ne => write!(f, "!="),
            ContractBinOp::Lt => write!(f, "<"),
            ContractBinOp::Le => write!(f, "<="),
            ContractBinOp::Gt => write!(f, ">"),
            ContractBinOp::Ge => write!(f, ">="),
            ContractBinOp::And => write!(f, "&&"),
            ContractBinOp::Or => write!(f, "||"),
            ContractBinOp::Imply => write!(f, "=>"),
        }
    }
}

/// Unary operators for contract expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContractUnOp {
    /// Logical negation
    Not,
    /// Arithmetic negation
    Neg,
}

impl fmt::Display for ContractUnOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContractUnOp::Not => write!(f, "!"),
            ContractUnOp::Neg => write!(f, "-"),
        }
    }
}

/// Captures the pre-state value of an expression.
///
/// In postconditions, `old(expr)` refers to the value of `expr` at function entry.
/// The compiler stores this value before executing the function body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OldExpr {
    /// The expression whose pre-state value is captured
    pub inner: Box<ContractExpr>,
    /// Generated storage variable name (for codegen)
    storage_var: Option<Text>,
}

impl OldExpr {
    /// Create a new old expression.
    pub fn new(inner: ContractExpr) -> Self {
        Self {
            inner: Box::new(inner),
            storage_var: None,
        }
    }

    /// Get the storage variable name for this old expression.
    ///
    /// Generates a unique name like `__old_0`, `__old_1`, etc.
    pub fn storage_name(&self) -> Text {
        if let Some(ref name) = self.storage_var {
            name.clone()
        } else {
            // Generate a default name based on the inner expression
            Text::from(format!("__old_{}", self.inner))
        }
    }

    /// Set the storage variable name.
    pub fn set_storage_var(&mut self, name: Text) {
        self.storage_var = Some(name);
    }
}

impl fmt::Display for OldExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "old({})", self.inner)
    }
}

/// Quantifier variable binding.
///
/// Supports both unbounded quantification (forall x. P(x)) and
/// bounded quantification over ranges (forall x in 0..N. P(x)).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuantifierBinding {
    /// The bound variable name
    pub variable: Text,
    /// Optional range bound (for bounded quantification)
    pub range: Option<QuantifierRange>,
    /// Optional type annotation
    pub var_type: Option<Text>,
}

impl QuantifierBinding {
    /// Create an unbounded quantifier binding.
    pub fn unbounded(variable: Text) -> Self {
        Self {
            variable,
            range: None,
            var_type: None,
        }
    }

    /// Create a bounded quantifier binding with a range.
    pub fn bounded(variable: Text, lower: ContractExpr, upper: ContractExpr) -> Self {
        Self {
            variable,
            range: Some(QuantifierRange {
                lower: Box::new(lower),
                upper: Box::new(upper),
                inclusive: false,
            }),
            var_type: None,
        }
    }

    /// Create with a type annotation.
    pub fn with_type(variable: Text, var_type: Text) -> Self {
        Self {
            variable,
            range: None,
            var_type: Some(var_type),
        }
    }
}

impl fmt::Display for QuantifierBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref range) = self.range {
            write!(f, "{} in {}..{}", self.variable, range.lower, range.upper)
        } else if let Some(ref ty) = self.var_type {
            write!(f, "{}: {}", self.variable, ty)
        } else {
            write!(f, "{}", self.variable)
        }
    }
}

/// Range for bounded quantification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuantifierRange {
    /// Lower bound (inclusive)
    pub lower: Box<ContractExpr>,
    /// Upper bound
    pub upper: Box<ContractExpr>,
    /// Whether upper bound is inclusive
    pub inclusive: bool,
}

// =============================================================================
// Contract Parser
// =============================================================================

/// Parser for contract#"..." literals.
///
/// Parses the contract DSL syntax from contract#"..." string literals.
/// This is a compiler intrinsic parser -- unlike user-defined tagged literals,
/// it is NOT registered via @tagged_literal and has deep integration with
/// the type system and SMT solver.
///
/// # Grammar
///
/// ```text
/// contract      ::= clause*
/// clause        ::= ('requires' | 'ensures' | 'invariant') expr ';'
/// expr          ::= logical_or
/// logical_or    ::= logical_and ('||' logical_and)*
/// logical_and   ::= comparison ('&&' comparison)*
/// comparison    ::= implication (cmp_op implication)?
/// implication   ::= additive ('=>' implication)?
/// additive      ::= multiplicative (('+' | '-') multiplicative)*
/// multiplicative ::= unary (('*' | '/' | '%') unary)*
/// unary         ::= ('!' | '-')? postfix
/// postfix       ::= primary ('.' ident | '[' expr ']' | '(' args ')' | '.' ident '(' args ')')*
/// primary       ::= 'true' | 'false' | number | ident | 'result' | 'old' '(' expr ')'
///                 | 'forall' binding '.' expr | 'exists' binding '.' expr
///                 | 'let' ident '=' expr 'in' expr | 'if' expr 'then' expr 'else' expr
///                 | '(' expr ')'
/// binding       ::= ident (':' type)? ('in' range)?
/// range         ::= expr '..' expr
/// ```
#[derive(Debug)]
pub struct ContractParser {
    /// Input text
    input: Text,
    /// Current position
    pos: usize,
    /// Source span for error reporting
    span: Span,
}

impl ContractParser {
    /// Create a new contract parser.
    pub fn new(input: Text, span: Span) -> Self {
        Self {
            input,
            pos: 0,
            span,
        }
    }

    /// Parse a complete contract specification.
    pub fn parse(&mut self) -> Result<ContractSpec, ContractError> {
        let mut spec = ContractSpec::new(self.span);

        self.skip_whitespace_and_comments();

        while !self.is_eof() {
            let clause_kind = self.parse_clause_keyword()?;
            self.skip_whitespace_and_comments();

            let expr = self.parse_expr()?;
            let pred = Predicate::new(expr, self.span);

            match clause_kind {
                ClauseKind::Requires => spec.preconditions.push(pred),
                ClauseKind::Ensures => spec.postconditions.push(pred),
                ClauseKind::Invariant => spec.invariants.push(pred),
            }

            self.skip_whitespace_and_comments();

            // Optional semicolon
            if self.peek_char() == Some(';') {
                self.advance();
            }

            self.skip_whitespace_and_comments();
        }

        // Validate the contract
        spec.validate()?;

        Ok(spec)
    }

    /// Parse a contract specification without validation.
    /// Useful for testing validation logic separately.
    pub fn parse_only(&mut self) -> Result<ContractSpec, ContractError> {
        let mut spec = ContractSpec::new(self.span);

        self.skip_whitespace_and_comments();

        while !self.is_eof() {
            let clause_kind = self.parse_clause_keyword()?;
            self.skip_whitespace_and_comments();

            let expr = self.parse_expr()?;
            let predicate = Predicate::new(expr, self.span);

            match clause_kind {
                ClauseKind::Requires => spec.preconditions.push(predicate),
                ClauseKind::Ensures => spec.postconditions.push(predicate),
                ClauseKind::Invariant => spec.invariants.push(predicate),
            }

            self.skip_whitespace_and_comments();

            // Optional semicolon
            if self.peek_char() == Some(';') {
                self.advance();
            }

            self.skip_whitespace_and_comments();
        }

        // NO validation - return parsed spec directly
        Ok(spec)
    }

    fn parse_clause_keyword(&mut self) -> Result<ClauseKind, ContractError> {
        if self.consume_keyword("requires") {
            Ok(ClauseKind::Requires)
        } else if self.consume_keyword("ensures") {
            Ok(ClauseKind::Ensures)
        } else if self.consume_keyword("invariant") {
            Ok(ClauseKind::Invariant)
        } else {
            Err(ContractError::ParseError {
                message: format!(
                    "expected 'requires', 'ensures', or 'invariant', found '{}'",
                    self.peek_word().unwrap_or_default()
                ),
                pos: self.pos,
                span: self.span,
            })
        }
    }

    fn parse_expr(&mut self) -> Result<ContractExpr, ContractError> {
        self.parse_implication_expr()
    }

    /// Parse implication expressions (lowest precedence binary operator)
    fn parse_implication_expr(&mut self) -> Result<ContractExpr, ContractError> {
        let mut left = self.parse_logical_or()?;

        while self.peek_op() == Some("=>") || self.peek_op() == Some("==>") {
            let op = if self.peek_op() == Some("==>") {
                "==>"
            } else {
                "=>"
            };
            self.consume_op(op);
            self.skip_whitespace_and_comments();
            let right = self.parse_logical_or()?;
            left = ContractExpr::BinOp(ContractBinOp::Imply, Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    fn parse_logical_or(&mut self) -> Result<ContractExpr, ContractError> {
        let mut left = self.parse_logical_and()?;

        while self.peek_op() == Some("||") {
            self.consume_op("||");
            self.skip_whitespace_and_comments();
            let right = self.parse_logical_and()?;
            left = ContractExpr::BinOp(ContractBinOp::Or, Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    fn parse_logical_and(&mut self) -> Result<ContractExpr, ContractError> {
        let mut left = self.parse_comparison()?;

        while self.peek_op() == Some("&&") {
            self.consume_op("&&");
            self.skip_whitespace_and_comments();
            let right = self.parse_comparison()?;
            left = ContractExpr::BinOp(ContractBinOp::And, Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<ContractExpr, ContractError> {
        let mut left = self.parse_additive()?;

        if let Some(op_str) = self.peek_comparison_op() {
            let op = match op_str {
                "==" => ContractBinOp::Eq,
                "!=" => ContractBinOp::Ne,
                "<=" => ContractBinOp::Le,
                ">=" => ContractBinOp::Ge,
                "<" => ContractBinOp::Lt,
                ">" => ContractBinOp::Gt,
                _ => unreachable!(),
            };
            self.consume_op(op_str);
            self.skip_whitespace_and_comments();
            let right = self.parse_additive()?;
            left = ContractExpr::BinOp(op, Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<ContractExpr, ContractError> {
        let mut left = self.parse_multiplicative()?;
        self.skip_whitespace_and_comments();

        while let Some(op_str) = self.peek_additive_op() {
            let op = match op_str {
                "+" => ContractBinOp::Add,
                "-" => ContractBinOp::Sub,
                _ => unreachable!(),
            };
            self.consume_op(op_str);
            self.skip_whitespace_and_comments();
            let right = self.parse_multiplicative()?;
            self.skip_whitespace_and_comments();
            left = ContractExpr::BinOp(op, Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<ContractExpr, ContractError> {
        let mut left = self.parse_unary()?;
        self.skip_whitespace_and_comments();

        while let Some(op_str) = self.peek_multiplicative_op() {
            let op = match op_str {
                "*" => ContractBinOp::Mul,
                "/" => ContractBinOp::Div,
                "%" => ContractBinOp::Mod,
                _ => unreachable!(),
            };
            self.consume_op(op_str);
            self.skip_whitespace_and_comments();
            let right = self.parse_unary()?;
            self.skip_whitespace_and_comments();
            left = ContractExpr::BinOp(op, Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<ContractExpr, ContractError> {
        if self.peek_char() == Some('!') {
            self.advance();
            self.skip_whitespace_and_comments();
            let expr = self.parse_unary()?;
            return Ok(ContractExpr::UnOp(ContractUnOp::Not, Box::new(expr)));
        }

        if self.peek_char() == Some('-') && !self.is_subtraction() {
            self.advance();
            self.skip_whitespace_and_comments();
            let expr = self.parse_unary()?;
            return Ok(ContractExpr::UnOp(ContractUnOp::Neg, Box::new(expr)));
        }

        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<ContractExpr, ContractError> {
        let mut expr = self.parse_primary()?;

        loop {
            self.skip_whitespace_and_comments();

            if self.peek_char() == Some('.') {
                self.advance();
                let field = self.parse_identifier()?;
                self.skip_whitespace_and_comments();

                if self.peek_char() == Some('(') {
                    // Method call
                    self.advance();
                    let args = self.parse_args()?;
                    self.expect_char(')')?;
                    expr = ContractExpr::MethodCall(Box::new(expr), field, args);
                } else {
                    // Field access
                    expr = ContractExpr::Field(Box::new(expr), field);
                }
            } else if self.peek_char() == Some('[') {
                // Index
                self.advance();
                self.skip_whitespace_and_comments();
                let index = self.parse_expr()?;
                self.skip_whitespace_and_comments();
                self.expect_char(']')?;
                expr = ContractExpr::Index(Box::new(expr), Box::new(index));
            } else if self.peek_char() == Some('(') {
                // Function call (when the primary was an identifier)
                self.advance();
                let args = self.parse_args()?;
                self.expect_char(')')?;

                if let ContractExpr::Var(name) = expr {
                    expr = ContractExpr::Call(name, args);
                } else {
                    return Err(ContractError::ParseError {
                        message: String::from("expected function name before '('"),
                        pos: self.pos,
                        span: self.span,
                    });
                }
            } else {
                break;
            }
        }

        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<ContractExpr, ContractError> {
        self.skip_whitespace_and_comments();

        // Parenthesized expression
        if self.peek_char() == Some('(') {
            self.advance();
            self.skip_whitespace_and_comments();
            let expr = self.parse_expr()?;
            self.skip_whitespace_and_comments();
            self.expect_char(')')?;
            return Ok(ContractExpr::Paren(Box::new(expr)));
        }

        // Number
        if let Some(c) = self.peek_char()
            && c.is_ascii_digit()
        {
            return self.parse_number();
        }

        // Keyword or identifier
        if let Some(word) = self.peek_word() {
            match word.as_str() {
                "true" => {
                    self.consume_word(word.as_str());
                    return Ok(ContractExpr::True);
                }
                "false" => {
                    self.consume_word(word.as_str());
                    return Ok(ContractExpr::False);
                }
                "result" => {
                    self.consume_word(word.as_str());
                    return Ok(ContractExpr::Result);
                }
                "old" => {
                    self.consume_word(word.as_str());
                    self.skip_whitespace_and_comments();
                    self.expect_char('(')?;
                    self.skip_whitespace_and_comments();
                    let inner = self.parse_expr()?;
                    self.skip_whitespace_and_comments();
                    self.expect_char(')')?;
                    return Ok(ContractExpr::Old(OldExpr::new(inner)));
                }
                "forall" => {
                    self.consume_word(word.as_str());
                    self.skip_whitespace_and_comments();
                    let binding = self.parse_quantifier_binding()?;
                    self.skip_whitespace_and_comments();
                    self.expect_char('.')?;
                    self.skip_whitespace_and_comments();
                    let body = self.parse_expr()?;
                    return Ok(ContractExpr::Forall(binding, Box::new(body)));
                }
                "exists" => {
                    self.consume_word(word.as_str());
                    self.skip_whitespace_and_comments();
                    let binding = self.parse_quantifier_binding()?;
                    self.skip_whitespace_and_comments();
                    self.expect_char('.')?;
                    self.skip_whitespace_and_comments();
                    let body = self.parse_expr()?;
                    return Ok(ContractExpr::Exists(binding, Box::new(body)));
                }
                "let" => {
                    self.consume_word(word.as_str());
                    self.skip_whitespace_and_comments();
                    let name = self.parse_identifier()?;
                    self.skip_whitespace_and_comments();
                    self.expect_char('=')?;
                    self.skip_whitespace_and_comments();
                    let val = self.parse_expr()?;
                    self.skip_whitespace_and_comments();
                    self.expect_keyword("in")?;
                    self.skip_whitespace_and_comments();
                    let body = self.parse_expr()?;
                    return Ok(ContractExpr::Let(name, Box::new(val), Box::new(body)));
                }
                "if" => {
                    self.consume_word(word.as_str());
                    self.skip_whitespace_and_comments();
                    let cond = self.parse_expr()?;
                    self.skip_whitespace_and_comments();
                    self.expect_keyword("then")?;
                    self.skip_whitespace_and_comments();
                    let then_expr = self.parse_expr()?;
                    self.skip_whitespace_and_comments();
                    self.expect_keyword("else")?;
                    self.skip_whitespace_and_comments();
                    let else_expr = self.parse_expr()?;
                    return Ok(ContractExpr::IfThenElse(
                        Box::new(cond),
                        Box::new(then_expr),
                        Box::new(else_expr),
                    ));
                }
                "len" => {
                    // Special function: len(arr)
                    self.consume_word(word.as_str());
                    self.skip_whitespace_and_comments();
                    self.expect_char('(')?;
                    let args = self.parse_args()?;
                    self.expect_char(')')?;
                    return Ok(ContractExpr::Call(Text::from("len"), args));
                }
                _ => {
                    // Regular identifier
                    let ident = self.parse_identifier()?;
                    return Ok(ContractExpr::Var(ident));
                }
            }
        }

        Err(ContractError::ParseError {
            message: format!(
                "unexpected character '{}'",
                self.peek_char().unwrap_or('\0')
            ),
            pos: self.pos,
            span: self.span,
        })
    }

    fn parse_quantifier_binding(&mut self) -> Result<QuantifierBinding, ContractError> {
        let var_name = self.parse_identifier()?;
        self.skip_whitespace_and_comments();

        // Check for type annotation
        let var_type = if self.peek_char() == Some(':') {
            self.advance();
            self.skip_whitespace_and_comments();
            Some(self.parse_identifier()?)
        } else {
            None
        };

        self.skip_whitespace_and_comments();

        // Check for range
        let range = if self.consume_keyword("in") {
            self.skip_whitespace_and_comments();
            let lower = self.parse_range_bound()?;
            self.skip_whitespace_and_comments();

            // Expect '..'
            if !self.consume_op("..") {
                return Err(ContractError::ParseError {
                    message: String::from("expected '..' in range"),
                    pos: self.pos,
                    span: self.span,
                });
            }
            self.skip_whitespace_and_comments();

            let upper = self.parse_range_bound()?;

            Some(QuantifierRange {
                lower: Box::new(lower),
                upper: Box::new(upper),
                inclusive: false,
            })
        } else {
            None
        };

        Ok(QuantifierBinding {
            variable: var_name,
            range,
            var_type,
        })
    }

    /// Parse a range bound expression (restricted to prevent consuming the '.' delimiter).
    /// Allows: numbers, identifiers, parenthesized expressions, and basic arithmetic.
    fn parse_range_bound(&mut self) -> Result<ContractExpr, ContractError> {
        let mut result = self.parse_range_primary()?;
        self.skip_whitespace_and_comments();

        // Allow basic arithmetic operations (+ - * / %)
        loop {
            let op = match self.peek_char() {
                Some('+') => {
                    self.advance();
                    ContractBinOp::Add
                }
                Some('-') => {
                    self.advance();
                    ContractBinOp::Sub
                }
                Some('*') => {
                    self.advance();
                    ContractBinOp::Mul
                }
                Some('/') => {
                    self.advance();
                    ContractBinOp::Div
                }
                Some('%') => {
                    self.advance();
                    ContractBinOp::Mod
                }
                _ => break,
            };
            self.skip_whitespace_and_comments();
            let right = self.parse_range_primary()?;
            result = ContractExpr::BinOp(op, Box::new(result), Box::new(right));
            self.skip_whitespace_and_comments();
        }

        Ok(result)
    }

    /// Parse a primary expression for range bounds (no field access allowed).
    ///
    /// This uses parse_integer instead of parse_number to avoid consuming
    /// the '.' that serves as the quantifier body separator.
    fn parse_range_primary(&mut self) -> Result<ContractExpr, ContractError> {
        self.skip_whitespace_and_comments();

        if let Some(c) = self.peek_char() {
            if c.is_ascii_digit() {
                // Use integer-only parsing in range context to avoid
                // consuming the '.' body separator as a decimal point
                return self.parse_integer();
            }
            if c == '(' {
                self.advance();
                self.skip_whitespace_and_comments();
                let inner = self.parse_range_bound()?;
                self.skip_whitespace_and_comments();
                self.expect_char(')')?;
                return Ok(ContractExpr::Paren(Box::new(inner)));
            }
            if c.is_alphabetic() || c == '_' {
                return self.parse_identifier().map(ContractExpr::Var);
            }
        }

        Err(ContractError::ParseError {
            message: String::from(
                "expected number, identifier, or parenthesized expression in range",
            ),
            pos: self.pos,
            span: self.span,
        })
    }

    /// Parse an integer literal only (no floats allowed).
    /// Used in range bounds to avoid consuming the '.' body separator.
    fn parse_integer(&mut self) -> Result<ContractExpr, ContractError> {
        let start = self.pos;

        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }

        let num_str = &self.input[start..self.pos];

        if num_str.is_empty() {
            return Err(ContractError::ParseError {
                message: String::from("expected integer literal"),
                pos: start,
                span: self.span,
            });
        }

        let value = num_str
            .parse::<i64>()
            .map_err(|_| ContractError::ParseError {
                message: format!("invalid integer literal: {}", num_str),
                pos: start,
                span: self.span,
            })?;

        Ok(ContractExpr::Int(value))
    }

    fn parse_args(&mut self) -> Result<List<ContractExpr>, ContractError> {
        let mut args = List::new();
        self.skip_whitespace_and_comments();

        if self.peek_char() == Some(')') {
            return Ok(args);
        }

        loop {
            args.push(self.parse_expr()?);
            self.skip_whitespace_and_comments();

            if self.peek_char() == Some(',') {
                self.advance();
                self.skip_whitespace_and_comments();
            } else {
                break;
            }
        }

        Ok(args)
    }

    fn parse_number(&mut self) -> Result<ContractExpr, ContractError> {
        let start = self.pos;
        let mut has_dot = false;

        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() {
                self.advance();
            } else if c == '.' && !has_dot {
                // Look ahead to distinguish from range operator
                let next = self.input[self.pos + 1..].chars().next();
                if next == Some('.') {
                    break; // This is a range operator
                }
                has_dot = true;
                self.advance();
            } else {
                break;
            }
        }

        let num_str = &self.input[start..self.pos];

        if has_dot {
            let value = num_str
                .parse::<f64>()
                .map_err(|_| ContractError::ParseError {
                    message: format!("invalid float literal: {}", num_str),
                    pos: start,
                    span: self.span,
                })?;
            Ok(ContractExpr::Float(value))
        } else {
            let value = num_str
                .parse::<i64>()
                .map_err(|_| ContractError::ParseError {
                    message: format!("invalid integer literal: {}", num_str),
                    pos: start,
                    span: self.span,
                })?;
            Ok(ContractExpr::Int(value))
        }
    }

    fn parse_identifier(&mut self) -> Result<Text, ContractError> {
        let start = self.pos;

        if let Some(c) = self.peek_char() {
            if !c.is_alphabetic() && c != '_' {
                return Err(ContractError::ParseError {
                    message: format!("expected identifier, found '{}'", c),
                    pos: self.pos,
                    span: self.span,
                });
            }
            self.advance();
        } else {
            return Err(ContractError::ParseError {
                message: String::from("unexpected end of input"),
                pos: self.pos,
                span: self.span,
            });
        }

        while let Some(c) = self.peek_char() {
            if c.is_alphanumeric() || c == '_' {
                self.advance();
            } else {
                break;
            }
        }

        Ok(Text::from(&self.input[start..self.pos]))
    }

    // Helper methods

    fn peek_char(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn advance(&mut self) {
        if let Some(c) = self.peek_char() {
            self.pos += c.len_utf8();
        }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn skip_whitespace_and_comments(&mut self) {
        while let Some(c) = self.peek_char() {
            if c.is_whitespace() {
                self.advance();
            } else if c == '/' && self.input[self.pos..].starts_with("//") {
                // Line comment
                while let Some(c) = self.peek_char() {
                    self.advance();
                    if c == '\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn peek_word(&self) -> Option<Text> {
        let start = self.pos;
        let mut end = start;

        for c in self.input[start..].chars() {
            if c.is_alphanumeric() || c == '_' {
                end += c.len_utf8();
            } else {
                break;
            }
        }

        if end > start {
            Some(Text::from(&self.input[start..end]))
        } else {
            None
        }
    }

    fn consume_word(&mut self, word: &str) {
        self.pos += word.len();
    }

    fn consume_keyword(&mut self, keyword: &str) -> bool {
        if self.input[self.pos..].starts_with(keyword) {
            let next_pos = self.pos + keyword.len();
            if next_pos >= self.input.len()
                || !self.input[next_pos..]
                    .chars()
                    .next()
                    .unwrap()
                    .is_alphanumeric()
            {
                self.pos = next_pos;
                return true;
            }
        }
        false
    }

    fn expect_char(&mut self, expected: char) -> Result<(), ContractError> {
        if self.peek_char() == Some(expected) {
            self.advance();
            Ok(())
        } else {
            Err(ContractError::ParseError {
                message: format!(
                    "expected '{}', found '{}'",
                    expected,
                    self.peek_char().unwrap_or('\0')
                ),
                pos: self.pos,
                span: self.span,
            })
        }
    }

    fn expect_keyword(&mut self, keyword: &str) -> Result<(), ContractError> {
        if self.consume_keyword(keyword) {
            Ok(())
        } else {
            Err(ContractError::ParseError {
                message: format!(
                    "expected '{}', found '{}'",
                    keyword,
                    self.peek_word().unwrap_or_default()
                ),
                pos: self.pos,
                span: self.span,
            })
        }
    }

    fn peek_op(&self) -> Option<&'static str> {
        let ops = ["==>", "=>", "==", "!=", "<=", ">=", "&&", "||", ".."];
        for op in &ops {
            if self.input[self.pos..].starts_with(op) {
                return Some(op);
            }
        }
        None
    }

    fn peek_comparison_op(&self) -> Option<&'static str> {
        let ops = ["==", "!=", "<=", ">=", "<", ">"];
        for op in &ops {
            if self.input[self.pos..].starts_with(op) {
                return Some(op);
            }
        }
        None
    }

    fn peek_additive_op(&self) -> Option<&'static str> {
        if self.input[self.pos..].starts_with('+') {
            Some("+")
        } else if self.input[self.pos..].starts_with('-') && self.is_subtraction() {
            Some("-")
        } else {
            None
        }
    }

    fn peek_multiplicative_op(&self) -> Option<&'static str> {
        if self.input[self.pos..].starts_with('*') {
            Some("*")
        } else if self.input[self.pos..].starts_with('/') {
            Some("/")
        } else if self.input[self.pos..].starts_with('%') {
            Some("%")
        } else {
            None
        }
    }

    fn is_subtraction(&self) -> bool {
        // Check if '-' is a subtraction operator (not unary minus)
        // Subtraction typically follows an expression (number, identifier, ')' or ']')
        // We need to skip back over whitespace to find the actual previous token
        if self.pos == 0 {
            return false;
        }

        // Look back, skipping whitespace
        let prev_chars = &self.input[..self.pos];
        let mut chars_rev = prev_chars.chars().rev().peekable();

        // Skip any whitespace
        while let Some(ch) = chars_rev.next() {
            if !ch.is_whitespace() {
                // Found non-whitespace - check if it's an expression ending
                // ')' or ']' always end an expression
                if matches!(ch, ')' | ']') {
                    return true;
                }
                // Digit ends a number expression
                if ch.is_ascii_digit() {
                    return true;
                }
                // For alphanumeric, we need to check if it's a keyword
                // Keywords like "requires", "ensures", "forall", "exists", "in", etc.
                // are NOT followed by subtraction - they're followed by unary minus
                if ch.is_alphanumeric() || ch == '_' {
                    // Extract the full identifier/keyword going backwards
                    let mut word = String::new();
                    word.push(ch);
                    for prev_ch in chars_rev {
                        if prev_ch.is_alphanumeric() || prev_ch == '_' {
                            word.push(prev_ch);
                        } else {
                            break;
                        }
                    }
                    // Reverse to get actual word
                    let word: String = word.chars().rev().collect();

                    // Check if it's a keyword - keywords are NOT expressions
                    // so after them we have unary minus, not subtraction
                    let keywords = [
                        "requires",
                        "ensures",
                        "invariant",
                        "decreases",
                        "forall",
                        "exists",
                        "in",
                        "true",
                        "false",
                        "old",
                        "result",
                        "len",
                        "if",
                        "then",
                        "else",
                        "let",
                        "and",
                        "or",
                        "not",
                        "mod",
                    ];

                    if keywords.contains(&word.as_str()) {
                        return false; // Keyword - unary minus
                    }

                    // It's a regular identifier - subtraction
                    return true;
                }
                // Any other character - probably not an expression ending
                return false;
            }
        }

        // Only whitespace before, so this is unary minus
        false
    }

    fn consume_op(&mut self, op: &str) -> bool {
        if self.input[self.pos..].starts_with(op) {
            self.pos += op.len();
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ClauseKind {
    Requires,
    Ensures,
    Invariant,
}

// =============================================================================
// SMT Translation
// =============================================================================

/// Translates a ContractSpec to SMT-LIB format for verification.
///
/// Phase 2 of contract verification: lower function body to SSA, generate
/// verification conditions (VC = forall params. preconditions => wp(body, postconditions)),
/// and encode in SMT-LIB 2.0 format for Z3/CVC5 solving.
#[derive(Debug)]
pub struct ContractSmtTranslator {
    /// Variable declarations
    declarations: List<(Text, VarType)>,
    /// Generated SMT assertions
    assertions: List<Text>,
}

impl ContractSmtTranslator {
    /// Create a new translator.
    pub fn new() -> Self {
        Self {
            declarations: List::new(),
            assertions: List::new(),
        }
    }

    /// Translate a contract spec to SMT-LIB format.
    ///
    /// Returns a complete SMT-LIB 2.6 script.
    pub fn translate(&mut self, spec: &ContractSpec, function_name: &str) -> Text {
        let mut output = Text::new();

        // Header
        output.push_str("; Contract Verification for ");
        output.push_str(function_name);
        output.push_str("\n");
        output.push_str("; Generated by Verum Contract System\n\n");
        output.push_str("(set-logic ALL)\n\n");

        // Collect free variables
        let mut all_vars = HashSet::new();
        for pred in spec.preconditions.iter() {
            all_vars.extend(pred.expr.free_variables());
        }
        for pred in spec.postconditions.iter() {
            all_vars.extend(pred.expr.free_variables());
        }
        for pred in spec.invariants.iter() {
            all_vars.extend(pred.expr.free_variables());
        }

        // Declare variables
        output.push_str("; Variable declarations\n");
        for var in &all_vars {
            output.push_str(&format!("(declare-const {} Int)\n", var));
        }
        output.push_str("(declare-const result Int)\n");

        // Declare old values
        let old_exprs = spec.collect_old_expressions();
        for old in old_exprs.iter() {
            let old_name = old.storage_name();
            output.push_str(&format!("(declare-const {} Int)\n", old_name));
        }
        output.push_str("\n");

        // Assert preconditions (assumed to hold)
        output.push_str("; Preconditions (assumed)\n");
        for (i, pred) in spec.preconditions.iter().enumerate() {
            let formula = pred.to_formula();
            output.push_str(&format!(
                "(assert {} ) ; precondition {}\n",
                formula.to_smtlib(),
                i + 1
            ));
        }
        output.push_str("\n");

        // Assert invariants
        output.push_str("; Invariants (assumed)\n");
        for (i, pred) in spec.invariants.iter().enumerate() {
            let formula = pred.to_formula();
            output.push_str(&format!(
                "(assert {} ) ; invariant {}\n",
                formula.to_smtlib(),
                i + 1
            ));
        }
        output.push_str("\n");

        // Negate postconditions (looking for counterexample)
        output.push_str("; Postconditions (negated for SAT check)\n");
        output.push_str("; If UNSAT, postconditions are proven\n");

        if spec.postconditions.len() == 1 {
            let pred = spec.postconditions.first().unwrap();
            let formula = pred.to_formula();
            output.push_str(&format!("(assert (not {} ))\n", formula.to_smtlib()));
        } else if !spec.postconditions.is_empty() {
            // Multiple postconditions: negate their conjunction
            let mut conj = Text::from("(and ");
            for pred in spec.postconditions.iter() {
                let formula = pred.to_formula();
                conj.push_str(formula.to_smtlib().as_str());
                conj.push(' ');
            }
            conj.push(')');
            output.push_str(&format!("(assert (not {}))\n", conj));
        }
        output.push_str("\n");

        // Check satisfiability
        output.push_str("(check-sat)\n");
        output.push_str("(get-model)\n");

        output
    }

    /// Generate verification conditions from a contract.
    pub fn generate_vcs(
        &self,
        spec: &ContractSpec,
        function_name: &str,
    ) -> List<VerificationCondition> {
        let mut vcs = List::new();

        // Combine preconditions
        let precondition = if spec.preconditions.is_empty() {
            Formula::True
        } else {
            Formula::and(spec.preconditions.iter().map(|p| p.to_formula()))
        };

        // Generate VC for each postcondition
        for (i, post) in spec.postconditions.iter().enumerate() {
            let post_formula = post.to_formula();

            // VC: Precondition => Postcondition
            let vc_formula = Formula::implies(precondition.clone(), post_formula);

            let vc = VerificationCondition::new(
                vc_formula,
                SourceLocation::from_span(spec.span, Text::from(function_name)),
                VCKind::Postcondition,
                format!("{} postcondition #{}", function_name, i + 1),
            )
            .with_function(function_name);

            vcs.push(vc);
        }

        // Generate VCs for invariants
        for (i, inv) in spec.invariants.iter().enumerate() {
            let inv_formula = inv.to_formula();

            let vc = VerificationCondition::new(
                inv_formula,
                SourceLocation::from_span(spec.span, Text::from(function_name)),
                VCKind::LoopInvariantInit,
                format!("{} invariant #{}", function_name, i + 1),
            )
            .with_function(function_name);

            vcs.push(vc);
        }

        vcs
    }
}

impl Default for ContractSmtTranslator {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Runtime Instrumentation
// =============================================================================

/// Generates runtime assertion code for @verify(runtime) mode.
///
/// For @verify(runtime) mode: converts contracts to runtime assertions.
/// Preconditions become entry checks, old() values are captured before body
/// execution, and postconditions become exit checks. Violations produce panics
/// with the clause text, source location, and actual values.
#[derive(Debug)]
pub struct RuntimeInstrumenter {
    /// Counter for generating unique old value variable names
    old_counter: u64,
}

impl RuntimeInstrumenter {
    /// Create a new runtime instrumenter.
    pub fn new() -> Self {
        Self { old_counter: 0 }
    }

    /// Generate instrumentation code for a contract.
    ///
    /// Returns:
    /// 1. Precondition checks (inserted at function entry)
    /// 2. Old value storage statements
    /// 3. Postcondition checks (inserted at function exit)
    pub fn instrument(&mut self, spec: &ContractSpec, function_name: &str) -> InstrumentedContract {
        let mut precondition_checks = List::new();
        let mut old_value_stores = List::new();
        let mut postcondition_checks = List::new();

        // Generate precondition checks
        for (i, pred) in spec.preconditions.iter().enumerate() {
            let check_code = self.generate_precondition_check(pred, function_name, i + 1);
            precondition_checks.push(check_code);
        }

        // Collect and generate old value storage
        let old_exprs = spec.collect_old_expressions();
        let mut old_mappings = Map::new();

        for mut old_expr in old_exprs.into_iter() {
            let storage_name = Text::from(format!("__old_{}", self.old_counter));
            self.old_counter += 1;

            let store_code = self.generate_old_value_store(&old_expr, &storage_name);
            old_mappings.insert(
                Text::from(format!("{}", old_expr.inner)),
                storage_name.clone(),
            );
            old_expr.set_storage_var(storage_name);
            old_value_stores.push(store_code);
        }

        // Generate postcondition checks
        for (i, pred) in spec.postconditions.iter().enumerate() {
            let check_code =
                self.generate_postcondition_check(pred, function_name, i + 1, &old_mappings);
            postcondition_checks.push(check_code);
        }

        InstrumentedContract {
            precondition_checks,
            old_value_stores,
            postcondition_checks,
        }
    }

    fn generate_precondition_check(
        &self,
        pred: &Predicate,
        function_name: &str,
        index: usize,
    ) -> Text {
        let condition = self.expr_to_code(&pred.expr);

        Text::from(format!(
            r#"if !({}) {{
    panic!(
        "Contract violation: Precondition #{} failed\n\
         Function: {}\n\
         Clause: requires {}\n"
    );
}}"#,
            condition, index, function_name, pred.expr
        ))
    }

    fn generate_old_value_store(&self, old_expr: &OldExpr, storage_name: &Text) -> Text {
        let value_code = self.expr_to_code(&old_expr.inner);
        Text::from(format!("let {} = {};", storage_name, value_code))
    }

    fn generate_postcondition_check(
        &self,
        pred: &Predicate,
        function_name: &str,
        index: usize,
        old_mappings: &Map<Text, Text>,
    ) -> Text {
        let condition = self.expr_to_code_with_old(&pred.expr, old_mappings);

        Text::from(format!(
            r#"if !({}) {{
    panic!(
        "Contract violation: Postcondition #{} failed\n\
         Function: {}\n\
         Clause: ensures {}\n"
    );
}}"#,
            condition, index, function_name, pred.expr
        ))
    }

    fn expr_to_code(&self, expr: &ContractExpr) -> Text {
        self.expr_to_code_with_old(expr, &Map::new())
    }

    fn expr_to_code_with_old(&self, expr: &ContractExpr, old_mappings: &Map<Text, Text>) -> Text {
        match expr {
            ContractExpr::True => Text::from("true"),
            ContractExpr::False => Text::from("false"),
            ContractExpr::Int(n) => Text::from(format!("{}", n)),
            ContractExpr::Float(f) => Text::from(format!("{}", f)),
            ContractExpr::Var(name) => name.clone(),
            ContractExpr::Result => Text::from("result"),
            ContractExpr::Old(old) => {
                // Look up the storage variable
                let key = Text::from(format!("{}", old.inner));
                if let Some(storage_name) = old_mappings.get(&key) {
                    storage_name.clone()
                } else {
                    old.storage_name().clone()
                }
            }
            ContractExpr::BinOp(op, l, r) => {
                let left = self.expr_to_code_with_old(l, old_mappings);
                let right = self.expr_to_code_with_old(r, old_mappings);
                let op_str = match op {
                    ContractBinOp::Add => "+",
                    ContractBinOp::Sub => "-",
                    ContractBinOp::Mul => "*",
                    ContractBinOp::Div => "/",
                    ContractBinOp::Mod => "%",
                    ContractBinOp::Eq => "==",
                    ContractBinOp::Ne => "!=",
                    ContractBinOp::Lt => "<",
                    ContractBinOp::Le => "<=",
                    ContractBinOp::Gt => ">",
                    ContractBinOp::Ge => ">=",
                    ContractBinOp::And => "&&",
                    ContractBinOp::Or => "||",
                    ContractBinOp::Imply => "||", // a => b is !a || b
                };

                if matches!(op, ContractBinOp::Imply) {
                    Text::from(format!("(!({}) || ({}))", left, right))
                } else {
                    Text::from(format!("({} {} {})", left, op_str, right))
                }
            }
            ContractExpr::UnOp(op, e) => {
                let inner = self.expr_to_code_with_old(e, old_mappings);
                match op {
                    ContractUnOp::Not => Text::from(format!("!({})", inner)),
                    ContractUnOp::Neg => Text::from(format!("-({})", inner)),
                }
            }
            ContractExpr::Call(name, args) => {
                let args_str: List<Text> = args
                    .iter()
                    .map(|a| self.expr_to_code_with_old(a, old_mappings))
                    .collect();
                Text::from(format!("{}({})", name, args_str.join(", ")))
            }
            ContractExpr::Field(e, field) => {
                let base = self.expr_to_code_with_old(e, old_mappings);
                Text::from(format!("{}.{}", base, field))
            }
            ContractExpr::Index(arr, idx) => {
                let arr_str = self.expr_to_code_with_old(arr, old_mappings);
                let idx_str = self.expr_to_code_with_old(idx, old_mappings);
                Text::from(format!("{}[{}]", arr_str, idx_str))
            }
            ContractExpr::MethodCall(recv, method, args) => {
                let recv_str = self.expr_to_code_with_old(recv, old_mappings);
                let args_str: List<Text> = args
                    .iter()
                    .map(|a| self.expr_to_code_with_old(a, old_mappings))
                    .collect();
                Text::from(format!("{}.{}({})", recv_str, method, args_str.join(", ")))
            }
            ContractExpr::Forall(binding, body) => {
                // Runtime forall: iterate over range
                if let Some(ref range) = binding.range {
                    let lo = self.expr_to_code_with_old(&range.lower, old_mappings);
                    let hi = self.expr_to_code_with_old(&range.upper, old_mappings);
                    let body_str = self.expr_to_code_with_old(body, old_mappings);
                    Text::from(format!(
                        "({}..{}).all(|{}| {})",
                        lo, hi, binding.variable, body_str
                    ))
                } else {
                    // Unbounded forall cannot be checked at runtime
                    Text::from(format!(
                        "/* forall {} - cannot check at runtime */ true",
                        binding.variable
                    ))
                }
            }
            ContractExpr::Exists(binding, body) => {
                // Runtime exists: iterate over range
                if let Some(ref range) = binding.range {
                    let lo = self.expr_to_code_with_old(&range.lower, old_mappings);
                    let hi = self.expr_to_code_with_old(&range.upper, old_mappings);
                    let body_str = self.expr_to_code_with_old(body, old_mappings);
                    Text::from(format!(
                        "({}..{}).any(|{}| {})",
                        lo, hi, binding.variable, body_str
                    ))
                } else {
                    Text::from(format!(
                        "/* exists {} - cannot check at runtime */ false",
                        binding.variable
                    ))
                }
            }
            ContractExpr::Let(name, val, body) => {
                let val_str = self.expr_to_code_with_old(val, old_mappings);
                let body_str = self.expr_to_code_with_old(body, old_mappings);
                Text::from(format!("{{ let {} = {}; {} }}", name, val_str, body_str))
            }
            ContractExpr::IfThenElse(c, t, e) => {
                let cond = self.expr_to_code_with_old(c, old_mappings);
                let then_str = self.expr_to_code_with_old(t, old_mappings);
                let else_str = self.expr_to_code_with_old(e, old_mappings);
                Text::from(format!(
                    "if {} {{ {} }} else {{ {} }}",
                    cond, then_str, else_str
                ))
            }
            ContractExpr::Paren(e) => {
                let inner = self.expr_to_code_with_old(e, old_mappings);
                Text::from(format!("({})", inner))
            }
        }
    }
}

impl Default for RuntimeInstrumenter {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of contract instrumentation for runtime checking.
#[derive(Debug, Clone)]
pub struct InstrumentedContract {
    /// Code to check preconditions at function entry
    pub precondition_checks: List<Text>,
    /// Code to store old() values before function body
    pub old_value_stores: List<Text>,
    /// Code to check postconditions at function exit
    pub postcondition_checks: List<Text>,
}

impl InstrumentedContract {
    /// Check if any instrumentation code was generated.
    pub fn is_empty(&self) -> bool {
        self.precondition_checks.is_empty()
            && self.old_value_stores.is_empty()
            && self.postcondition_checks.is_empty()
    }

    /// Generate the complete instrumentation code as a string.
    pub fn to_code(&self) -> Text {
        let mut code = Text::new();

        // Precondition checks
        for check in self.precondition_checks.iter() {
            code.push_str(check.as_str());
            code.push_str("\n\n");
        }

        // Old value stores
        for store in self.old_value_stores.iter() {
            code.push_str(store.as_str());
            code.push('\n');
        }
        if !self.old_value_stores.is_empty() {
            code.push('\n');
        }

        // Postcondition checks (marker for where to insert)
        code.push_str("// --- Function body here ---\n\n");

        for check in self.postcondition_checks.iter() {
            code.push_str(check.as_str());
            code.push_str("\n\n");
        }

        code
    }
}

// =============================================================================
// Error Types
// =============================================================================

/// Errors that can occur during contract handling.
#[derive(Debug, Error)]
pub enum ContractError {
    /// Parse error
    #[error("parse error at position {pos}: {message}")]
    ParseError {
        message: String,
        pos: usize,
        span: Span,
    },

    /// Invalid usage of contract features
    #[error("invalid contract usage: {message}")]
    InvalidUsage { message: String, span: Span },

    /// Verification failed
    #[error("verification failed: {message}")]
    VerificationFailed {
        message: String,
        counterexample: Option<Map<Text, Text>>,
    },

    /// SMT translation error
    #[error("SMT translation error: {0}")]
    SmtError(Text),

    /// Internal error
    #[error("internal error: {0}")]
    Internal(Text),
}

// =============================================================================
// Public API Functions
// =============================================================================

/// Parse a contract#"..." literal into a ContractSpec.
///
/// This is the main entry point for parsing contract literals.
///
/// # Example
///
/// ```rust,ignore
/// use verum_verification::contract::parse_contract;
/// use verum_ast::span::Span;
///
/// let content = "requires x > 0; ensures result >= 0;";
/// let spec = parse_contract(content, Span::dummy())?;
/// assert_eq!(spec.preconditions.len(), 1);
/// assert_eq!(spec.postconditions.len(), 1);
/// ```
pub fn parse_contract(content: &str, span: Span) -> Result<ContractSpec, ContractError> {
    let mut parser = ContractParser::new(Text::from(content), span);
    parser.parse()
}

/// Parse a contract literal without validation.
///
/// This is primarily intended for testing validation separately from parsing.
/// For production use, prefer `parse_contract` which includes validation.
pub fn parse_contract_no_validate(
    content: &str,
    span: Span,
) -> Result<ContractSpec, ContractError> {
    let mut parser = ContractParser::new(Text::from(content), span);
    parser.parse_only()
}

/// Translate a contract specification to SMT-LIB format.
///
/// Returns a complete SMT-LIB 2.6 script that can be sent to a solver.
pub fn contract_to_smtlib(spec: &ContractSpec, function_name: &str) -> Text {
    let mut translator = ContractSmtTranslator::new();
    translator.translate(spec, function_name)
}

/// Generate verification conditions from a contract.
pub fn generate_contract_vcs(
    spec: &ContractSpec,
    function_name: &str,
) -> List<VerificationCondition> {
    let translator = ContractSmtTranslator::new();
    translator.generate_vcs(spec, function_name)
}

/// Generate runtime instrumentation code for a contract.
///
/// For use with @verify(runtime) mode.
pub fn instrument_contract(spec: &ContractSpec, function_name: &str) -> InstrumentedContract {
    let mut instrumenter = RuntimeInstrumenter::new();
    instrumenter.instrument(spec, function_name)
}

/// Validate a contract specification for semantic correctness.
pub fn validate_contract(spec: &ContractSpec) -> Result<(), ContractError> {
    spec.validate()
}
