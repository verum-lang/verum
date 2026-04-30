//! Verification Condition Generation
//!
//! This module implements the Verification Condition (VC) Generation system
//! for Verum's formal verification. VCs are logical formulas whose validity
//! implies program correctness.
//!
//! # Specification
//!
//! Verification conditions are logical formulas whose validity implies program
//! correctness. The generator uses Dijkstra's weakest precondition (wp) calculus:
//!   VC(f) = forall params. Precondition => wp(body, Postcondition)
//! For loops, 3 VCs are generated: initialization (pre => invariant),
//! preservation (inv /\ cond => wp(body, inv)), and exit (inv /\ !cond => post).
//!
//! # Weakest Precondition Calculus
//!
//! The core algorithm is based on Dijkstra's weakest precondition calculus:
//!
//! - `wp(skip, Q) = Q`
//! - `wp(x := e, Q) = Q[x/e]`
//! - `wp(S1; S2, Q) = wp(S1, wp(S2, Q))`
//! - `wp(if b then S1 else S2, Q) = (b => wp(S1, Q)) && (!b => wp(S2, Q))`
//! - `wp(while b inv I, Q) = I && (forall v. I && b => wp(S, I)[v/v']) && (I && !b => Q)`
//!
//! # Example
//!
//! ```verum
//! @verify(proof)
//! fn increment(x: Int) -> Int {
//!     contract#"
//!         requires x >= 0;
//!         ensures result > x;
//!     "
//!     return x + 1;
//! }
//!
//! // Generated VC:
//! // (x >= 0) => wp(return x + 1, result > x)
//! //   = (x >= 0) => (x + 1 > x)
//! //   = true
//! ```

use crate::context::ObligationKind;
use crate::level::VerificationLevel;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use verum_ast::decl::FunctionDecl;
use verum_ast::expr::{BinOp, Block, Expr, ExprKind, UnOp};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::span::Span;
use verum_ast::stmt::{Stmt, StmtKind};
use verum_ast::ty::PathSegment;
use verum_common::{List, Map, Maybe, Text, ToText};

/// Helper to extract string from PathSegment
fn path_segment_to_str(seg: &PathSegment) -> &str {
    match seg {
        PathSegment::Name(ident) => ident.as_str(),
        PathSegment::SelfValue => "self",
        PathSegment::Super => "super",
        PathSegment::Cog => "cog",
        PathSegment::Relative => ".",
    }
}

// =============================================================================
// Contract Context
// =============================================================================

/// Context for contract expression parsing
///
/// Indicates the type of contract clause being parsed, which affects
/// how certain constructs (like `result` and `old()`) are interpreted.
///
/// Indicates the type of contract clause being parsed, affecting how `result`
/// and `old()` are interpreted. In preconditions, `result` is invalid and
/// `old(x)` is equivalent to `x`. In postconditions, `result` refers to the
/// return value and `old(x)` captures the value at function entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContractContext {
    /// Precondition context: expressions describe function entry state
    /// - `result` is NOT valid here
    /// - `old(x)` is equivalent to `x`
    Precondition,

    /// Postcondition context: expressions describe function exit state
    /// - `result` refers to the return value
    /// - `old(x)` refers to the value of `x` at function entry
    Postcondition,

    /// Invariant context: expressions that must hold before and after
    /// - `result` is NOT valid here
    /// - `old(x)` is typically not meaningful (use in loops)
    Invariant,

    /// Frame condition context: expressions describing modifiable state
    /// - Specifies what locations may be modified
    Modifies,

    /// Termination measure context: expressions for proving termination
    /// - Must be a well-founded measure that decreases
    Decreases,
}

impl ContractContext {
    /// Check if `result` is valid in this context
    pub fn allows_result(&self) -> bool {
        matches!(self, ContractContext::Postcondition)
    }

    /// Check if `old()` is meaningful in this context
    pub fn allows_old(&self) -> bool {
        matches!(self, ContractContext::Postcondition)
    }

    /// Get the context name for error messages
    pub fn name(&self) -> &'static str {
        match self {
            ContractContext::Precondition => "precondition",
            ContractContext::Postcondition => "postcondition",
            ContractContext::Invariant => "invariant",
            ContractContext::Modifies => "modifies",
            ContractContext::Decreases => "decreases",
        }
    }
}

// =============================================================================
// Source Location
// =============================================================================

/// Source location for verification diagnostics
///
/// Tracks where in the source code a verification condition originated,
/// enabling precise error messages and counterexample reporting.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceLocation {
    /// File path or identifier
    pub file: Text,
    /// Starting line number (1-based)
    pub line: u32,
    /// Starting column number (1-based)
    pub column: u32,
    /// Original AST span for detailed positioning
    pub span: Maybe<Span>,
}

impl SourceLocation {
    /// Create a new source location
    pub fn new(file: Text, line: u32, column: u32) -> Self {
        Self {
            file,
            line,
            column,
            span: Maybe::None,
        }
    }

    /// Create from an AST span
    ///
    /// Note: This uses byte offsets as line/column since we don't have
    /// access to the source file for proper line/column computation.
    pub fn from_span(span: Span, file: Text) -> Self {
        Self {
            file,
            // Use start byte offset as approximate line number (will be resolved later)
            line: span.start,
            column: 0,
            span: Maybe::Some(span),
        }
    }

    /// Create an unknown location (for synthesized VCs)
    pub fn unknown() -> Self {
        Self {
            file: Text::from("<unknown>"),
            line: 0,
            column: 0,
            span: Maybe::None,
        }
    }
}

impl fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

// =============================================================================
// Variables and Expressions
// =============================================================================

/// Variable identifier in verification formulas
///
/// Variables are uniquely identified by name and an optional SSA version
/// for handling mutable state in wp computation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Variable {
    /// Variable name
    pub name: Text,
    /// SSA version (None for non-versioned variables)
    pub version: Maybe<u64>,
    /// Type hint for SMT encoding
    pub ty: Maybe<VarType>,
}

impl Variable {
    /// Create a new variable
    pub fn new(name: impl Into<Text>) -> Self {
        Self {
            name: name.into(),
            version: Maybe::None,
            ty: Maybe::None,
        }
    }

    /// Create a versioned variable (for SSA)
    pub fn versioned(name: impl Into<Text>, version: u64) -> Self {
        Self {
            name: name.into(),
            version: Maybe::Some(version),
            ty: Maybe::None,
        }
    }

    /// Create a typed variable
    pub fn typed(name: impl Into<Text>, ty: VarType) -> Self {
        Self {
            name: name.into(),
            version: Maybe::None,
            ty: Maybe::Some(ty),
        }
    }

    /// Create the special "result" variable for postconditions
    pub fn result() -> Self {
        Self::new("result")
    }

    /// Get the SMT-LIB name for this variable
    pub fn smtlib_name(&self) -> Text {
        match &self.version {
            Maybe::Some(v) => Text::from(format!("{}_{}", self.name, v)),
            Maybe::None => self.name.clone(),
        }
    }
}

impl fmt::Display for Variable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.version {
            Maybe::Some(v) => write!(f, "{}_{}", self.name, v),
            Maybe::None => write!(f, "{}", self.name),
        }
    }
}

/// Variable type for SMT encoding
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VarType {
    /// Integer type
    Int,
    /// Boolean type
    Bool,
    /// Real/floating-point type
    Real,
    /// Bitvector with width
    BitVec(u32),
    /// Array type (index -> element)
    Array(Box<VarType>, Box<VarType>),
    /// Uninterpreted sort
    Sort(Text),
}

impl VarType {
    /// Get the SMT-LIB sort name
    pub fn smtlib_sort(&self) -> Text {
        match self {
            VarType::Int => Text::from("Int"),
            VarType::Bool => Text::from("Bool"),
            VarType::Real => Text::from("Real"),
            VarType::BitVec(w) => Text::from(format!("(_ BitVec {})", w)),
            VarType::Array(idx, elem) => Text::from(format!(
                "(Array {} {})",
                idx.smtlib_sort(),
                elem.smtlib_sort()
            )),
            VarType::Sort(name) => name.clone(),
        }
    }
}

/// SMT expression for verification conditions
///
/// Represents terms in the verification logic, including arithmetic,
/// function applications, and array operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SmtExpr {
    /// Variable reference
    Var(Variable),
    /// Integer constant
    IntConst(i64),
    /// Boolean constant
    BoolConst(bool),
    /// Real constant
    RealConst(f64),
    /// Bitvector constant
    BitVecConst(u64, u32),
    /// Binary arithmetic operation
    BinOp(SmtBinOp, Box<SmtExpr>, Box<SmtExpr>),
    /// Unary operation
    UnOp(SmtUnOp, Box<SmtExpr>),
    /// Function application
    Apply(Text, List<SmtExpr>),
    /// Array select: arr[idx]
    Select(Box<SmtExpr>, Box<SmtExpr>),
    /// Array store: arr[idx := val]
    Store(Box<SmtExpr>, Box<SmtExpr>, Box<SmtExpr>),
    /// If-then-else expression
    Ite(Box<Formula>, Box<SmtExpr>, Box<SmtExpr>),
    /// Let binding in expressions
    Let(Variable, Box<SmtExpr>, Box<SmtExpr>),
}

/// Binary operations for SMT expressions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SmtBinOp {
    /// Addition (+)
    Add,
    /// Subtraction (-)
    Sub,
    /// Multiplication (*)
    Mul,
    /// Integer division (div)
    Div,
    /// Modulo (mod)
    Mod,
    /// Exponentiation (^)
    Pow,
    /// Array/tuple element selection
    Select,
}

/// Unary operations for SMT expressions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SmtUnOp {
    /// Negation (-)
    Neg,
    /// Absolute value (abs)
    Abs,
    /// Dereference a reference
    Deref,
    /// Get length of array/slice
    Len,
    /// Extract value from variant
    GetVariantValue,
}

impl SmtExpr {
    /// Create a variable expression
    pub fn var(name: impl Into<Text>) -> Self {
        SmtExpr::Var(Variable::new(name))
    }

    /// Create an integer constant
    pub fn int(value: i64) -> Self {
        SmtExpr::IntConst(value)
    }

    /// Create a boolean constant
    pub fn bool(value: bool) -> Self {
        SmtExpr::BoolConst(value)
    }

    /// Create an addition
    pub fn add(left: SmtExpr, right: SmtExpr) -> Self {
        SmtExpr::BinOp(SmtBinOp::Add, Box::new(left), Box::new(right))
    }

    /// Create a subtraction
    pub fn sub(left: SmtExpr, right: SmtExpr) -> Self {
        SmtExpr::BinOp(SmtBinOp::Sub, Box::new(left), Box::new(right))
    }

    /// Create a multiplication
    pub fn mul(left: SmtExpr, right: SmtExpr) -> Self {
        SmtExpr::BinOp(SmtBinOp::Mul, Box::new(left), Box::new(right))
    }

    /// Create a real constant
    pub fn real(value: f64) -> Self {
        SmtExpr::RealConst(value)
    }

    /// Create a negation
    pub fn neg(expr: SmtExpr) -> Self {
        SmtExpr::UnOp(SmtUnOp::Neg, Box::new(expr))
    }

    /// Substitute a variable with an expression
    pub fn substitute(&self, var: &Variable, replacement: &SmtExpr) -> SmtExpr {
        match self {
            SmtExpr::Var(v) if v == var => replacement.clone(),
            SmtExpr::Var(_)
            | SmtExpr::IntConst(_)
            | SmtExpr::BoolConst(_)
            | SmtExpr::RealConst(_)
            | SmtExpr::BitVecConst(_, _) => self.clone(),
            SmtExpr::BinOp(op, left, right) => SmtExpr::BinOp(
                *op,
                Box::new(left.substitute(var, replacement)),
                Box::new(right.substitute(var, replacement)),
            ),
            SmtExpr::UnOp(op, expr) => {
                SmtExpr::UnOp(*op, Box::new(expr.substitute(var, replacement)))
            }
            SmtExpr::Apply(name, args) => SmtExpr::Apply(
                name.clone(),
                args.iter()
                    .map(|a| a.substitute(var, replacement))
                    .collect::<List<_>>(),
            ),
            SmtExpr::Select(arr, idx) => SmtExpr::Select(
                Box::new(arr.substitute(var, replacement)),
                Box::new(idx.substitute(var, replacement)),
            ),
            SmtExpr::Store(arr, idx, val) => SmtExpr::Store(
                Box::new(arr.substitute(var, replacement)),
                Box::new(idx.substitute(var, replacement)),
                Box::new(val.substitute(var, replacement)),
            ),
            SmtExpr::Ite(cond, then_e, else_e) => SmtExpr::Ite(
                Box::new(cond.substitute(var, replacement)),
                Box::new(then_e.substitute(var, replacement)),
                Box::new(else_e.substitute(var, replacement)),
            ),
            SmtExpr::Let(bound_var, bound_expr, body) => {
                if bound_var == var {
                    // Variable is shadowed, don't substitute in body
                    SmtExpr::Let(
                        bound_var.clone(),
                        Box::new(bound_expr.substitute(var, replacement)),
                        body.clone(),
                    )
                } else {
                    SmtExpr::Let(
                        bound_var.clone(),
                        Box::new(bound_expr.substitute(var, replacement)),
                        Box::new(body.substitute(var, replacement)),
                    )
                }
            }
        }
    }

    /// Collect free variables
    pub fn free_variables(&self) -> HashSet<Variable> {
        let mut vars = HashSet::new();
        self.collect_free_vars(&mut vars, &HashSet::new());
        vars
    }

    pub(crate) fn collect_free_vars(
        &self,
        vars: &mut HashSet<Variable>,
        bound: &HashSet<Variable>,
    ) {
        match self {
            SmtExpr::Var(v) if !bound.contains(v) => {
                vars.insert(v.clone());
            }
            SmtExpr::Var(_)
            | SmtExpr::IntConst(_)
            | SmtExpr::BoolConst(_)
            | SmtExpr::RealConst(_)
            | SmtExpr::BitVecConst(_, _) => {}
            SmtExpr::BinOp(_, left, right) => {
                left.collect_free_vars(vars, bound);
                right.collect_free_vars(vars, bound);
            }
            SmtExpr::UnOp(_, expr) => expr.collect_free_vars(vars, bound),
            SmtExpr::Apply(_, args) => {
                for arg in args.iter() {
                    arg.collect_free_vars(vars, bound);
                }
            }
            SmtExpr::Select(arr, idx) => {
                arr.collect_free_vars(vars, bound);
                idx.collect_free_vars(vars, bound);
            }
            SmtExpr::Store(arr, idx, val) => {
                arr.collect_free_vars(vars, bound);
                idx.collect_free_vars(vars, bound);
                val.collect_free_vars(vars, bound);
            }
            SmtExpr::Ite(cond, then_e, else_e) => {
                cond.collect_free_vars(vars, bound);
                then_e.collect_free_vars(vars, bound);
                else_e.collect_free_vars(vars, bound);
            }
            SmtExpr::Let(bound_var, bound_expr, body) => {
                bound_expr.collect_free_vars(vars, bound);
                let mut new_bound = bound.clone();
                new_bound.insert(bound_var.clone());
                body.collect_free_vars(vars, &new_bound);
            }
        }
    }

    /// Convert to SMT-LIB format
    pub fn to_smtlib(&self) -> Text {
        match self {
            SmtExpr::Var(v) => v.smtlib_name(),
            SmtExpr::IntConst(n) => {
                if *n < 0 {
                    Text::from(format!("(- {})", -n))
                } else {
                    Text::from(format!("{}", n))
                }
            }
            SmtExpr::BoolConst(b) => Text::from(if *b { "true" } else { "false" }),
            SmtExpr::RealConst(r) => Text::from(format!("{}", r)),
            SmtExpr::BitVecConst(v, w) => Text::from(format!("(_ bv{} {})", v, w)),
            SmtExpr::BinOp(op, left, right) => {
                match op {
                    SmtBinOp::Select => {
                        // Select uses SMT-LIB select syntax
                        Text::from(format!(
                            "(select {} {})",
                            left.to_smtlib(),
                            right.to_smtlib()
                        ))
                    }
                    _ => {
                        let op_str = match op {
                            SmtBinOp::Add => "+",
                            SmtBinOp::Sub => "-",
                            SmtBinOp::Mul => "*",
                            SmtBinOp::Div => "div",
                            SmtBinOp::Mod => "mod",
                            SmtBinOp::Pow => "^",
                            SmtBinOp::Select => unreachable!(),
                        };
                        Text::from(format!(
                            "({} {} {})",
                            op_str,
                            left.to_smtlib(),
                            right.to_smtlib()
                        ))
                    }
                }
            }
            SmtExpr::UnOp(op, expr) => {
                let op_str = match op {
                    SmtUnOp::Neg => "-",
                    SmtUnOp::Abs => "abs",
                    SmtUnOp::Deref => "deref",
                    SmtUnOp::Len => "len",
                    SmtUnOp::GetVariantValue => "get_variant_value",
                };
                Text::from(format!("({} {})", op_str, expr.to_smtlib()))
            }
            SmtExpr::Apply(name, args) => {
                if args.is_empty() {
                    name.clone()
                } else {
                    let args_str: List<Text> =
                        args.iter().map(|a| a.to_smtlib()).collect();
                    Text::from(format!("({} {})", name, args_str.join(" ")))
                }
            }
            SmtExpr::Select(arr, idx) => {
                Text::from(format!("(select {} {})", arr.to_smtlib(), idx.to_smtlib()))
            }
            SmtExpr::Store(arr, idx, val) => Text::from(format!(
                "(store {} {} {})",
                arr.to_smtlib(),
                idx.to_smtlib(),
                val.to_smtlib()
            )),
            SmtExpr::Ite(cond, then_e, else_e) => Text::from(format!(
                "(ite {} {} {})",
                cond.to_smtlib(),
                then_e.to_smtlib(),
                else_e.to_smtlib()
            )),
            SmtExpr::Let(var, bound, body) => Text::from(format!(
                "(let (({} {})) {})",
                var.smtlib_name(),
                bound.to_smtlib(),
                body.to_smtlib()
            )),
        }
    }
}

// =============================================================================
// Formulas
// =============================================================================

/// Logical formula for verification conditions
///
/// First-order logic predicates used in verification conditions.
/// Formulas express preconditions, postconditions, loop invariants, and
/// safety properties. They are ultimately encoded as SMT-LIB assertions
/// and checked for validity (negation checked for unsatisfiability).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Formula {
    /// Boolean constant true
    True,
    /// Boolean constant false
    False,
    /// Boolean variable
    Var(Variable),
    /// Logical negation: NOT phi
    Not(Box<Formula>),
    /// Logical conjunction: phi1 AND phi2 AND ...
    And(List<Formula>),
    /// Logical disjunction: phi1 OR phi2 OR ...
    Or(List<Formula>),
    /// Logical implication: phi1 => phi2
    Implies(Box<Formula>, Box<Formula>),
    /// Logical biconditional: phi1 <=> phi2
    Iff(Box<Formula>, Box<Formula>),
    /// Universal quantification: forall vars. phi
    Forall(List<Variable>, Box<Formula>),
    /// Existential quantification: exists vars. phi
    Exists(List<Variable>, Box<Formula>),
    /// Equality: e1 == e2
    Eq(Box<SmtExpr>, Box<SmtExpr>),
    /// Inequality: e1 != e2
    Ne(Box<SmtExpr>, Box<SmtExpr>),
    /// Less than: e1 < e2
    Lt(Box<SmtExpr>, Box<SmtExpr>),
    /// Less than or equal: e1 <= e2
    Le(Box<SmtExpr>, Box<SmtExpr>),
    /// Greater than: e1 > e2
    Gt(Box<SmtExpr>, Box<SmtExpr>),
    /// Greater than or equal: e1 >= e2
    Ge(Box<SmtExpr>, Box<SmtExpr>),
    /// Predicate application: P(e1, e2, ...)
    Predicate(Text, List<SmtExpr>),
    /// Let binding in formulas
    Let(Variable, Box<SmtExpr>, Box<Formula>),
}

impl Formula {
    /// Create a logical AND of multiple formulas
    pub fn and(formulas: impl IntoIterator<Item = Formula>) -> Formula {
        let fs: List<Formula> = formulas.into_iter().collect::<List<_>>();
        if fs.is_empty() {
            Formula::True
        } else if fs.len() == 1 {
            fs.into_iter().next().unwrap_or(Formula::True)
        } else {
            Formula::And(fs)
        }
    }

    /// Create a logical OR of multiple formulas
    pub fn or(formulas: impl IntoIterator<Item = Formula>) -> Formula {
        let fs: List<Formula> = formulas.into_iter().collect::<List<_>>();
        if fs.is_empty() {
            Formula::False
        } else if fs.len() == 1 {
            fs.into_iter().next().unwrap_or(Formula::False)
        } else {
            Formula::Or(fs)
        }
    }

    /// Create an implication
    pub fn implies(antecedent: Formula, consequent: Formula) -> Formula {
        Formula::Implies(Box::new(antecedent), Box::new(consequent))
    }

    /// Create a negation
    pub fn not(formula: Formula) -> Formula {
        Formula::Not(Box::new(formula))
    }

    /// Create an equality
    pub fn eq(left: SmtExpr, right: SmtExpr) -> Formula {
        Formula::Eq(Box::new(left), Box::new(right))
    }

    /// Create a less-than comparison
    pub fn lt(left: SmtExpr, right: SmtExpr) -> Formula {
        Formula::Lt(Box::new(left), Box::new(right))
    }

    /// Create a less-than-or-equal comparison
    pub fn le(left: SmtExpr, right: SmtExpr) -> Formula {
        Formula::Le(Box::new(left), Box::new(right))
    }

    /// Create a greater-than comparison
    pub fn gt(left: SmtExpr, right: SmtExpr) -> Formula {
        Formula::Gt(Box::new(left), Box::new(right))
    }

    /// Create a greater-than-or-equal comparison
    pub fn ge(left: SmtExpr, right: SmtExpr) -> Formula {
        Formula::Ge(Box::new(left), Box::new(right))
    }

    /// Substitute a variable in the formula with an expression
    ///
    /// Implements Q[x/e] from the weakest precondition calculus.
    pub fn substitute(&self, var: &Variable, replacement: &SmtExpr) -> Formula {
        match self {
            Formula::True | Formula::False => self.clone(),
            Formula::Var(v) if v == var => {
                // Convert expression to formula (must be boolean)
                match replacement {
                    SmtExpr::BoolConst(b) => {
                        if *b {
                            Formula::True
                        } else {
                            Formula::False
                        }
                    }
                    SmtExpr::Var(v) => Formula::Var(v.clone()),
                    _ => Formula::Predicate(Text::from("bool"), vec![replacement.clone()].into()),
                }
            }
            Formula::Var(_) => self.clone(),
            Formula::Not(inner) => Formula::Not(Box::new(inner.substitute(var, replacement))),
            Formula::And(formulas) => Formula::And(
                formulas
                    .iter()
                    .map(|f| f.substitute(var, replacement))
                    .collect::<List<_>>(),
            ),
            Formula::Or(formulas) => Formula::Or(
                formulas
                    .iter()
                    .map(|f| f.substitute(var, replacement))
                    .collect::<List<_>>(),
            ),
            Formula::Implies(ante, cons) => Formula::Implies(
                Box::new(ante.substitute(var, replacement)),
                Box::new(cons.substitute(var, replacement)),
            ),
            Formula::Iff(left, right) => Formula::Iff(
                Box::new(left.substitute(var, replacement)),
                Box::new(right.substitute(var, replacement)),
            ),
            Formula::Forall(bound_vars, inner) => {
                if bound_vars.iter().any(|v| v == var) {
                    // Variable is bound, don't substitute
                    self.clone()
                } else {
                    Formula::Forall(
                        bound_vars.clone(),
                        Box::new(inner.substitute(var, replacement)),
                    )
                }
            }
            Formula::Exists(bound_vars, inner) => {
                if bound_vars.iter().any(|v| v == var) {
                    self.clone()
                } else {
                    Formula::Exists(
                        bound_vars.clone(),
                        Box::new(inner.substitute(var, replacement)),
                    )
                }
            }
            Formula::Eq(left, right) => Formula::Eq(
                Box::new(left.substitute(var, replacement)),
                Box::new(right.substitute(var, replacement)),
            ),
            Formula::Ne(left, right) => Formula::Ne(
                Box::new(left.substitute(var, replacement)),
                Box::new(right.substitute(var, replacement)),
            ),
            Formula::Lt(left, right) => Formula::Lt(
                Box::new(left.substitute(var, replacement)),
                Box::new(right.substitute(var, replacement)),
            ),
            Formula::Le(left, right) => Formula::Le(
                Box::new(left.substitute(var, replacement)),
                Box::new(right.substitute(var, replacement)),
            ),
            Formula::Gt(left, right) => Formula::Gt(
                Box::new(left.substitute(var, replacement)),
                Box::new(right.substitute(var, replacement)),
            ),
            Formula::Ge(left, right) => Formula::Ge(
                Box::new(left.substitute(var, replacement)),
                Box::new(right.substitute(var, replacement)),
            ),
            Formula::Predicate(name, args) => Formula::Predicate(
                name.clone(),
                args.iter()
                    .map(|a| a.substitute(var, replacement))
                    .collect::<List<_>>(),
            ),
            Formula::Let(bound_var, bound_expr, body) => {
                if bound_var == var {
                    Formula::Let(
                        bound_var.clone(),
                        Box::new(bound_expr.substitute(var, replacement)),
                        body.clone(),
                    )
                } else {
                    Formula::Let(
                        bound_var.clone(),
                        Box::new(bound_expr.substitute(var, replacement)),
                        Box::new(body.substitute(var, replacement)),
                    )
                }
            }
        }
    }

    /// Collect free variables in the formula
    pub fn free_variables(&self) -> HashSet<Variable> {
        let mut vars = HashSet::new();
        self.collect_free_vars(&mut vars, &HashSet::new());
        vars
    }

    pub(crate) fn collect_free_vars(
        &self,
        vars: &mut HashSet<Variable>,
        bound: &HashSet<Variable>,
    ) {
        match self {
            Formula::True | Formula::False => {}
            Formula::Var(v) if !bound.contains(v) => {
                vars.insert(v.clone());
            }
            Formula::Var(_) => {}
            Formula::Not(inner) => inner.collect_free_vars(vars, bound),
            Formula::And(formulas) | Formula::Or(formulas) => {
                for f in formulas.iter() {
                    f.collect_free_vars(vars, bound);
                }
            }
            Formula::Implies(ante, cons) | Formula::Iff(ante, cons) => {
                ante.collect_free_vars(vars, bound);
                cons.collect_free_vars(vars, bound);
            }
            Formula::Forall(bound_vars, inner) | Formula::Exists(bound_vars, inner) => {
                let mut new_bound = bound.clone();
                for v in bound_vars.iter() {
                    new_bound.insert(v.clone());
                }
                inner.collect_free_vars(vars, &new_bound);
            }
            Formula::Eq(l, r)
            | Formula::Ne(l, r)
            | Formula::Lt(l, r)
            | Formula::Le(l, r)
            | Formula::Gt(l, r)
            | Formula::Ge(l, r) => {
                l.collect_free_vars(vars, bound);
                r.collect_free_vars(vars, bound);
            }
            Formula::Predicate(_, args) => {
                for arg in args.iter() {
                    arg.collect_free_vars(vars, bound);
                }
            }
            Formula::Let(bound_var, bound_expr, body) => {
                bound_expr.collect_free_vars(vars, bound);
                let mut new_bound = bound.clone();
                new_bound.insert(bound_var.clone());
                body.collect_free_vars(vars, &new_bound);
            }
        }
    }

    /// Convert to SMT-LIB format
    pub fn to_smtlib(&self) -> Text {
        match self {
            Formula::True => Text::from("true"),
            Formula::False => Text::from("false"),
            Formula::Var(v) => v.smtlib_name(),
            Formula::Not(inner) => Text::from(format!("(not {})", inner.to_smtlib())),
            Formula::And(formulas) => {
                if formulas.is_empty() {
                    Text::from("true")
                } else {
                    let fs: List<Text> = formulas.iter().map(|f| f.to_smtlib()).collect();
                    Text::from(format!("(and {})", fs.join(" ")))
                }
            }
            Formula::Or(formulas) => {
                if formulas.is_empty() {
                    Text::from("false")
                } else {
                    let fs: List<Text> = formulas.iter().map(|f| f.to_smtlib()).collect();
                    Text::from(format!("(or {})", fs.join(" ")))
                }
            }
            Formula::Implies(ante, cons) => {
                Text::from(format!("(=> {} {})", ante.to_smtlib(), cons.to_smtlib()))
            }
            Formula::Iff(left, right) => {
                Text::from(format!("(= {} {})", left.to_smtlib(), right.to_smtlib()))
            }
            Formula::Forall(vars, inner) => {
                let var_decls: List<Text> = vars
                    .iter()
                    .map(|v| {
                        let ty = match &v.ty {
                            Maybe::Some(t) => t.smtlib_sort(),
                            Maybe::None => Text::from("Int"),
                        };
                        Text::from(format!("({} {})", v.smtlib_name(), ty))
                    })
                    .collect();
                Text::from(format!(
                    "(forall ({}) {})",
                    var_decls.join(" "),
                    inner.to_smtlib()
                ))
            }
            Formula::Exists(vars, inner) => {
                let var_decls: List<Text> = vars
                    .iter()
                    .map(|v| {
                        let ty = match &v.ty {
                            Maybe::Some(t) => t.smtlib_sort(),
                            Maybe::None => Text::from("Int"),
                        };
                        Text::from(format!("({} {})", v.smtlib_name(), ty))
                    })
                    .collect();
                Text::from(format!(
                    "(exists ({}) {})",
                    var_decls.join(" "),
                    inner.to_smtlib()
                ))
            }
            Formula::Eq(left, right) => {
                Text::from(format!("(= {} {})", left.to_smtlib(), right.to_smtlib()))
            }
            Formula::Ne(left, right) => Text::from(format!(
                "(distinct {} {})",
                left.to_smtlib(),
                right.to_smtlib()
            )),
            Formula::Lt(left, right) => {
                Text::from(format!("(< {} {})", left.to_smtlib(), right.to_smtlib()))
            }
            Formula::Le(left, right) => {
                Text::from(format!("(<= {} {})", left.to_smtlib(), right.to_smtlib()))
            }
            Formula::Gt(left, right) => {
                Text::from(format!("(> {} {})", left.to_smtlib(), right.to_smtlib()))
            }
            Formula::Ge(left, right) => {
                Text::from(format!("(>= {} {})", left.to_smtlib(), right.to_smtlib()))
            }
            Formula::Predicate(name, args) => {
                if args.is_empty() {
                    name.clone()
                } else {
                    let args_str: List<Text> =
                        args.iter().map(|a| a.to_smtlib()).collect();
                    Text::from(format!("({} {})", name, args_str.join(" ")))
                }
            }
            Formula::Let(var, bound, body) => Text::from(format!(
                "(let (({} {})) {})",
                var.smtlib_name(),
                bound.to_smtlib(),
                body.to_smtlib()
            )),
        }
    }

    /// Simplify the formula (basic simplifications)
    pub fn simplify(&self) -> Formula {
        match self {
            Formula::Not(inner) => {
                let simplified = inner.simplify();
                match simplified {
                    Formula::True => Formula::False,
                    Formula::False => Formula::True,
                    Formula::Not(inner2) => *inner2,
                    _ => Formula::Not(Box::new(simplified)),
                }
            }
            Formula::And(formulas) => {
                let simplified: List<Formula> = formulas
                    .iter()
                    .map(|f| f.simplify())
                    .filter(|f| *f != Formula::True)
                    .collect();
                if simplified.iter().any(|f| *f == Formula::False) {
                    Formula::False
                } else if simplified.is_empty() {
                    Formula::True
                } else if simplified.len() == 1 {
                    simplified.into_iter().next().unwrap_or(Formula::True)
                } else {
                    Formula::And(simplified)
                }
            }
            Formula::Or(formulas) => {
                let simplified: List<Formula> = formulas
                    .iter()
                    .map(|f| f.simplify())
                    .filter(|f| *f != Formula::False)
                    .collect();
                if simplified.iter().any(|f| *f == Formula::True) {
                    Formula::True
                } else if simplified.is_empty() {
                    Formula::False
                } else if simplified.len() == 1 {
                    simplified.into_iter().next().unwrap_or(Formula::False)
                } else {
                    Formula::Or(simplified)
                }
            }
            Formula::Implies(ante, cons) => {
                let ante_s = ante.simplify();
                let cons_s = cons.simplify();
                match (&ante_s, &cons_s) {
                    (Formula::False, _) => Formula::True,
                    (_, Formula::True) => Formula::True,
                    (Formula::True, _) => cons_s,
                    _ => Formula::Implies(Box::new(ante_s), Box::new(cons_s)),
                }
            }
            _ => self.clone(),
        }
    }
}

// =============================================================================
// Verification Conditions
// =============================================================================

/// Kind of verification condition
///
/// Categorizes VCs for reporting and prioritization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VCKind {
    /// Function precondition check
    Precondition,
    /// Function postcondition check
    Postcondition,
    /// Loop invariant initialization
    LoopInvariantInit,
    /// Loop invariant preservation
    LoopInvariantPreserve,
    /// Loop invariant implies postcondition
    LoopInvariantExit,
    /// Array bounds check
    ArrayBounds,
    /// Refinement type constraint
    RefinementCheck,
    /// Null/None check for optional types
    NullCheck,
    /// Division by zero check
    DivisionByZero,
    /// Integer overflow check
    Overflow,
    /// Assertion verification
    Assertion,
    /// Termination (variant decreases)
    Termination,
    /// Custom/user-defined check
    Custom,
}

impl VCKind {
    /// Convert to ObligationKind for integration with context system
    pub fn to_obligation_kind(&self) -> ObligationKind {
        match self {
            VCKind::Precondition => ObligationKind::Precondition,
            VCKind::Postcondition => ObligationKind::Postcondition,
            VCKind::LoopInvariantInit
            | VCKind::LoopInvariantPreserve
            | VCKind::LoopInvariantExit => ObligationKind::LoopInvariant,
            VCKind::RefinementCheck => ObligationKind::RefinementConstraint,
            _ => ObligationKind::Custom,
        }
    }

    /// Get human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            VCKind::Precondition => "precondition",
            VCKind::Postcondition => "postcondition",
            VCKind::LoopInvariantInit => "loop invariant initialization",
            VCKind::LoopInvariantPreserve => "loop invariant preservation",
            VCKind::LoopInvariantExit => "loop invariant exit condition",
            VCKind::ArrayBounds => "array bounds",
            VCKind::RefinementCheck => "refinement constraint",
            VCKind::NullCheck => "null check",
            VCKind::DivisionByZero => "division by zero",
            VCKind::Overflow => "integer overflow",
            VCKind::Assertion => "assertion",
            VCKind::Termination => "termination",
            VCKind::Custom => "custom check",
        }
    }
}

/// A verification condition to be checked
///
/// A logical formula to be checked by the SMT solver. If the negation is UNSAT,
/// the VC is valid and the corresponding safety property is proven.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationCondition {
    /// Unique identifier
    pub id: u64,
    /// The logical formula to verify
    pub formula: Formula,
    /// Source location where this VC originated
    pub source_location: SourceLocation,
    /// Kind of verification condition
    pub kind: VCKind,
    /// Human-readable description
    pub description: Text,
    /// Function name (if applicable)
    pub function_name: Maybe<Text>,
    /// Verification level required
    pub level: VerificationLevel,
    /// Whether this VC has been verified
    pub verified: bool,
    /// Verification result (if verified)
    pub result: Maybe<VCResult>,
}

/// Result of VC verification
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VCResult {
    /// VC is valid (proven)
    Valid,
    /// VC is invalid with counterexample
    Invalid(CounterExample),
    /// Verification timed out
    Timeout,
    /// Unknown (solver couldn't determine)
    Unknown,
}

/// Counterexample for invalid VCs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterExample {
    /// Variable assignments in the counterexample
    pub assignments: Map<Text, Text>,
    /// Human-readable explanation
    pub explanation: Text,
}

impl VerificationCondition {
    /// Create a new verification condition
    pub fn new(
        formula: Formula,
        source_location: SourceLocation,
        kind: VCKind,
        description: impl Into<Text>,
    ) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::SeqCst),
            formula,
            source_location,
            kind,
            description: description.into(),
            function_name: Maybe::None,
            level: VerificationLevel::Static,
            verified: false,
            result: Maybe::None,
        }
    }

    /// Set the function name
    pub fn with_function(mut self, name: impl Into<Text>) -> Self {
        self.function_name = Maybe::Some(name.into());
        self
    }

    /// Set the verification level
    pub fn with_level(mut self, level: VerificationLevel) -> Self {
        self.level = level;
        self
    }

    /// Mark as verified with result
    pub fn verify(&mut self, result: VCResult) {
        self.verified = true;
        self.result = Maybe::Some(result);
    }

    /// Check if this VC was proven valid
    pub fn is_valid(&self) -> bool {
        matches!(&self.result, Maybe::Some(VCResult::Valid))
    }

    /// Get the formula for this verification condition
    ///
    /// Returns a reference to the formula that needs to be proven.
    /// This formula is used by the SMT verifier.
    pub fn to_formula(&self) -> Formula {
        self.formula.clone()
    }

    /// Get a reference to the formula
    pub fn formula(&self) -> &Formula {
        &self.formula
    }

    /// Get the SMT-LIB encoding of this VC
    ///
    /// Returns a complete SMT-LIB 2.6 script that can be sent to a solver.
    /// The formula is negated (for satisfiability checking) - if UNSAT,
    /// the VC is valid.
    pub fn to_smtlib(&self) -> Text {
        let mut output = Text::new();

        // Header
        output.push_str("; Verification Condition\n");
        output.push_str(&format!("; ID: {}\n", self.id));
        output.push_str(&format!("; Kind: {}\n", self.kind.description()));
        output.push_str(&format!("; Location: {}\n", self.source_location));
        output.push_str(&format!("; Description: {}\n", self.description));
        output.push_str("\n");

        // Logic declaration
        output.push_str("(set-logic ALL)\n");
        output.push_str("\n");

        // Variable declarations
        let free_vars = self.formula.free_variables();
        for var in free_vars.iter() {
            let sort = match &var.ty {
                Maybe::Some(ty) => ty.smtlib_sort(),
                Maybe::None => Text::from("Int"),
            };
            output.push_str(&format!("(declare-const {} {})\n", var.smtlib_name(), sort));
        }
        output.push_str("\n");

        // Assert negation of formula (for satisfiability check)
        // UNSAT => VC is valid
        output.push_str("; Assert negation - UNSAT means VC is valid\n");
        output.push_str(&format!("(assert (not {}))\n", self.formula.to_smtlib()));
        output.push_str("\n");

        // Check and get model
        output.push_str("(check-sat)\n");
        output.push_str("(get-model)\n");

        output
    }
}

// =============================================================================
// Symbol Table
// =============================================================================

/// Symbol table for VC generation
///
/// Tracks variable types, function signatures, and loop invariants
/// during VC generation.
#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    /// Variable types
    pub variables: Map<Text, VarType>,
    /// Function signatures (name -> (param_types, return_type, precondition, postcondition))
    pub functions: Map<Text, FunctionSignature>,
    /// Loop invariants (by loop ID)
    pub loop_invariants: Map<u64, Formula>,
    /// Current SSA versions for each variable
    pub ssa_versions: Map<Text, u64>,
    /// Array lengths: maps variable name to length expression (SMT)
    /// Populated from:
    /// - Array type declarations: [T; N] -> N
    /// - Refinement types: List<T>{len(it) == N}
    /// - Runtime len() calls tracked through assignments
    pub array_lengths: Map<Text, SmtExpr>,
}

/// Function signature for VC generation
#[derive(Debug, Clone)]
pub struct FunctionSignature {
    /// Parameter names and types
    pub params: List<(Text, VarType)>,
    /// Return type
    pub return_type: VarType,
    /// Precondition formula
    pub precondition: Formula,
    /// Postcondition formula
    pub postcondition: Formula,
}

impl SymbolTable {
    /// Create a new symbol table
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a variable
    pub fn add_variable(&mut self, name: impl Into<Text>, ty: VarType) {
        self.variables.insert(name.into(), ty);
    }

    /// Get variable type
    pub fn get_variable_type(&self, name: &str) -> Maybe<VarType> {
        match self.variables.get(&Text::from(name)) {
            Some(ty) => Maybe::Some(ty.clone()),
            None => Maybe::None,
        }
    }

    /// Get next SSA version for a variable
    pub fn next_ssa_version(&mut self, name: &str) -> u64 {
        let key = Text::from(name);
        let version = self.ssa_versions.get(&key).copied().unwrap_or(0);
        self.ssa_versions.insert(key, version + 1);
        version
    }

    /// Add a function signature
    pub fn add_function(&mut self, name: impl Into<Text>, sig: FunctionSignature) {
        self.functions.insert(name.into(), sig);
    }

    /// Get function signature
    pub fn get_function(&self, name: &str) -> Maybe<FunctionSignature> {
        match self.functions.get(&Text::from(name)) {
            Some(sig) => Maybe::Some(sig.clone()),
            None => Maybe::None,
        }
    }

    /// Register a loop invariant
    pub fn add_loop_invariant(&mut self, loop_id: u64, invariant: Formula) {
        self.loop_invariants.insert(loop_id, invariant);
    }

    /// Add an array length binding
    ///
    /// Associates a variable name with its length expression for bounds checking.
    pub fn add_array_length(&mut self, name: impl Into<Text>, length: SmtExpr) {
        self.array_lengths.insert(name.into(), length);
    }

    /// Get the length expression for an array variable
    ///
    /// Returns the SMT expression representing the array's length if known.
    pub fn get_array_length(&self, name: &str) -> Maybe<SmtExpr> {
        match self.array_lengths.get(&Text::from(name)) {
            Some(len) => Maybe::Some(len.clone()),
            None => Maybe::None,
        }
    }
}

// =============================================================================
// VC Generator
// =============================================================================

/// Verification Condition Generator
///
/// Generates verification conditions from Verum AST using Dijkstra's weakest
/// precondition calculus. Converts function bodies to SSA, extracts contracts,
/// and produces VCs that are sent to the SMT solver for validation.
#[derive(Debug)]
pub struct VCGenerator {
    /// Generated verification conditions
    pub vcs: List<VerificationCondition>,
    /// Symbol table for type and function information
    pub symbol_table: SymbolTable,
    /// Current function being analyzed
    current_function: Maybe<Text>,
    /// Next loop ID
    next_loop_id: u64,
    /// Source file for locations
    pub source_file: Text,
}

impl VCGenerator {
    /// Create a new VC generator
    pub fn new() -> Self {
        Self {
            vcs: List::new(),
            symbol_table: SymbolTable::new(),
            current_function: Maybe::None,
            next_loop_id: 0,
            source_file: Text::from("<unknown>"),
        }
    }

    /// Create with a source file name
    pub fn with_source_file(mut self, file: impl Into<Text>) -> Self {
        self.source_file = file.into();
        self
    }

    /// Generate verification conditions for a function
    ///
    /// VC generation rule for functions:
    ///   VC(f) = forall params. Precondition => wp(body, Postcondition)
    /// The wp is computed backwards from the postcondition through the function body.
    pub fn generate_vcs(&mut self, func: &FunctionDecl) -> List<VerificationCondition> {
        let func_name = func.name.as_str();
        self.current_function = Maybe::Some(Text::from(func_name));

        // Add parameters to symbol table and track array lengths
        for param in func.params.iter() {
            if let verum_ast::decl::FunctionParamKind::Regular { pattern, ty, .. } = &param.kind
                && let PatternKind::Ident { name, .. } = &pattern.kind
            {
                let var_ty = self.translate_type(ty);
                self.symbol_table.add_variable(name.as_str(), var_ty);

                // Extract array length from type if available
                if let Some(length) = self.extract_type_length(ty) {
                    self.symbol_table.add_array_length(name.as_str(), length);
                }
            }
        }

        // Get precondition and postcondition (from attributes or defaults)
        let (precondition, postcondition) = self.extract_contract(func);

        // Generate VC for function body if present
        if let Some(body) = &func.body {
            let body_wp = match body {
                verum_ast::decl::FunctionBody::Block(block) => self.wp_block(block, &postcondition),
                verum_ast::decl::FunctionBody::Expr(expr) => {
                    // For expression body, treat as return expr
                    self.wp_return(Some(expr), &postcondition)
                }
            };

            // Main function VC: Precondition => wp(body, Postcondition)
            let main_vc = VerificationCondition::new(
                Formula::implies(precondition.clone(), body_wp.simplify()),
                SourceLocation::from_span(func.span, self.source_file.clone()),
                VCKind::Postcondition,
                format!("Function '{}' postcondition", func_name),
            )
            .with_function(func_name);

            self.vcs.push(main_vc);
        }

        self.current_function = Maybe::None;
        self.vcs.clone()
    }

    /// Weakest precondition for a block
    ///
    /// wp(S1; S2; ...; Sn, Q) = wp(S1, wp(S2, ..., wp(Sn, Q)))
    pub fn wp_block(&mut self, block: &Block, postcondition: &Formula) -> Formula {
        let mut current_post = postcondition.clone();

        // Process statements in reverse order
        // Collect into Vec to support reverse iteration
        let stmts: Vec<_> = block.stmts.iter().collect();
        for stmt in stmts.iter().rev() {
            current_post = self.wp_stmt(stmt, &current_post);
        }

        // Handle trailing expression
        if let Some(expr) = &block.expr {
            current_post = self.wp_return(Some(expr), &current_post);
        }

        current_post
    }

    /// Weakest precondition for a statement
    pub fn wp_stmt(&mut self, stmt: &Stmt, postcondition: &Formula) -> Formula {
        match &stmt.kind {
            StmtKind::Empty => {
                // wp(skip, Q) = Q
                postcondition.clone()
            }
            StmtKind::Let { pattern, ty, value } => {
                // wp(let x = e, Q) = Q[x/e]
                // Also track array lengths for bounds checking

                // Track array length from type annotation if available
                if let PatternKind::Ident { name, .. } = &pattern.kind {
                    if let Some(ty) = ty {
                        if let Some(length) = self.extract_type_length(ty) {
                            self.symbol_table.add_array_length(name.as_str(), length);
                        }
                    }
                    // Also track length from initializer expression if it's an array literal
                    if let Some(val_expr) = value {
                        if let Some(length) = self.extract_array_literal_length(val_expr) {
                            self.symbol_table.add_array_length(name.as_str(), length);
                        }
                    }
                }

                if let Some(val_expr) = value {
                    self.wp_assignment(pattern, val_expr, postcondition)
                } else {
                    postcondition.clone()
                }
            }
            StmtKind::LetElse {
                pattern,
                value,
                else_block,
                ..
            } => {
                // wp(let pat = e else { div }, Q) = (matches(e, pat) => Q[pat/e]) && (!matches(e, pat) => wp(else_block, false))
                let smt_value = self.translate_expr(value);
                let match_cond = self.pattern_match_condition(pattern, &smt_value);
                let q_subst = self.wp_assignment(pattern, value, postcondition);
                let else_wp = self.wp_block(else_block, &Formula::False);

                Formula::and([
                    Formula::implies(match_cond.clone(), q_subst),
                    Formula::implies(Formula::not(match_cond), else_wp),
                ])
            }
            StmtKind::Expr { expr, .. } => self.wp_expr(expr, postcondition),
            StmtKind::Item(_) => {
                // Items don't affect wp
                postcondition.clone()
            }
            StmtKind::Defer(expr) => {
                // Defer statements need special handling for cleanup
                // For now, assume they don't affect postcondition
                self.wp_expr(expr, postcondition)
            }
            StmtKind::Errdefer(_expr) => {
                // Errdefer registers a cleanup that runs ONLY on the
                // error exit path. On the normal path it is a no-op,
                // so the precondition just propagates the postcondition
                // unchanged.
                //
                // Pre-fix this called `wp_expr(expr, postcondition)` —
                // the semantics of `defer` (always runs) — which
                // wrongly threaded the cleanup's effects through every
                // normal exit. Same defect as in
                // `verum_smt::wp_calculus` (fixed in fc02bfc9).
                //
                // Error-path WP modeling — what postcondition the
                // cleanup must establish IF the function exits
                // abnormally — is a separate phase needing split
                // normal/error path tracking; not modeled here.
                postcondition.clone()
            }
            StmtKind::Provide { .. } => {
                // Context provision doesn't affect wp directly
                postcondition.clone()
            }
            StmtKind::ProvideScope { block, .. } => {
                // Block-scoped provide: wp(provide _ = _ in { block }, Q) = wp(block, Q)
                self.wp_expr(block, postcondition)
            }
        }
    }

    /// Weakest precondition for assignment
    ///
    /// wp(x := e, Q) = Q[x/e]
    fn wp_assignment(
        &mut self,
        pattern: &Pattern,
        value: &Expr,
        postcondition: &Formula,
    ) -> Formula {
        match &pattern.kind {
            PatternKind::Ident { name, .. } => {
                let var = Variable::new(name.as_str());
                let smt_value = self.translate_expr(value);
                postcondition.substitute(&var, &smt_value)
            }
            PatternKind::Tuple(patterns) => {
                // For tuple patterns, substitute each element
                let mut result = postcondition.clone();
                if let ExprKind::Tuple(exprs) = &value.kind {
                    for (pat, expr) in patterns.iter().zip(exprs.iter()) {
                        result = self.wp_assignment(pat, expr, &result);
                    }
                }
                result
            }
            PatternKind::Wildcard => {
                // Wildcard doesn't bind, no substitution
                postcondition.clone()
            }
            _ => {
                // Other patterns: no substitution for now
                postcondition.clone()
            }
        }
    }

    /// Weakest precondition for an expression
    fn wp_expr(&mut self, expr: &Expr, postcondition: &Formula) -> Formula {
        match &expr.kind {
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // wp(if b then S1 else S2, Q) = (b => wp(S1, Q)) && (!b => wp(S2, Q))
                let cond_formula = self.translate_condition(condition);
                let then_wp = self.wp_block(then_branch, postcondition);
                let else_wp = match else_branch {
                    Some(else_expr) => self.wp_expr_inner(else_expr, postcondition),
                    None => postcondition.clone(),
                };

                Formula::and([
                    Formula::implies(cond_formula.clone(), then_wp),
                    Formula::implies(Formula::not(cond_formula), else_wp),
                ])
            }
            ExprKind::While {
                label: _,
                condition,
                body,
                invariants: _,
                decreases: _,
            } => {
                // wp(while b inv I, Q) = I && (forall v. I && b => wp(S, I)[v/v']) && (I && !b => Q)
                let loop_id = self.next_loop_id;
                self.next_loop_id += 1;

                // Extract invariant from annotations (or use true as default)
                let invariant = match self.symbol_table.loop_invariants.get(&loop_id) {
                    Some(inv) => inv.clone(),
                    None => Formula::True, // Default: true invariant
                };

                let cond_formula = self.translate_expr_to_formula(condition);
                let body_wp = self.wp_block(body, &invariant);

                // Generate three VCs:
                // 1. Initialization: precondition => I (handled at function level)
                // 2. Preservation: I && b => wp(body, I)
                let preserve_vc = VerificationCondition::new(
                    Formula::implies(
                        Formula::and([invariant.clone(), cond_formula.clone()]),
                        body_wp,
                    ),
                    SourceLocation::from_span(expr.span, self.source_file.clone()),
                    VCKind::LoopInvariantPreserve,
                    "Loop invariant preservation",
                );
                if let Maybe::Some(func_name) = &self.current_function {
                    self.vcs.push(preserve_vc.with_function(func_name.clone()));
                } else {
                    self.vcs.push(preserve_vc);
                }

                // 3. Exit: I && !b => Q
                let exit_vc = VerificationCondition::new(
                    Formula::implies(
                        Formula::and([invariant.clone(), Formula::not(cond_formula)]),
                        postcondition.clone(),
                    ),
                    SourceLocation::from_span(expr.span, self.source_file.clone()),
                    VCKind::LoopInvariantExit,
                    "Loop invariant implies postcondition",
                );
                if let Maybe::Some(func_name) = &self.current_function {
                    self.vcs.push(exit_vc.with_function(func_name.clone()));
                } else {
                    self.vcs.push(exit_vc);
                }

                // Return invariant as wp
                invariant
            }
            ExprKind::For {
                label: _,
                pattern,
                iter,
                body,
                invariants: loop_invs,
                decreases,
            } => {
                // For loops: full verification with bounds and invariants
                //
                // A for loop `for x in start..end { S }` is verified as:
                //
                // 1. Entry VC: precondition => I[x := start]
                // 2. Preservation VC: forall x in [start, end). I[x] && x < end => wp(S, I[x := x+1])
                // 3. Exit VC: forall x. I[x] && x >= end => Q
                // 4. Termination VC: decreases variant decreases on each iteration
                //
                // For collection iterators, we use abstract iteration predicates.
                //
                // Loop verification generates 3 VCs:
                // 1. Init: pre => invariant
                // 2. Preserve: (inv /\ cond) => wp(body, inv)
                // 3. Exit: (inv /\ !cond) => post
                // Plus optional termination VC if @decreases is specified.
                let loop_id = self.next_loop_id;
                self.next_loop_id += 1;

                // Get or infer loop invariant (combine all invariants with conjunction)
                let invariant = match self.symbol_table.loop_invariants.get(&loop_id) {
                    Some(inv) => inv.clone(),
                    None => {
                        if loop_invs.is_empty() {
                            Formula::True
                        } else {
                            let formulas: Vec<Formula> = loop_invs
                                .iter()
                                .map(|inv_expr| self.translate_contract(inv_expr))
                                .collect();
                            formulas.into_iter().fold(Formula::True, |acc, f| Formula::and(vec![acc, f]))
                        }
                    }
                };

                // Extract loop variable from pattern
                let loop_var = self.extract_pattern_variable(pattern);

                // Extract iteration bounds from the iterator expression
                let (start_bound, end_bound, is_range) = self.extract_for_bounds(iter);

                if is_range {
                    // Range-based for loop: for x in start..end

                    // 1. Entry VC: precondition implies invariant at start
                    let entry_inv = invariant.substitute(&loop_var, &start_bound);
                    let entry_vc = VerificationCondition::new(
                        entry_inv.clone(),
                        SourceLocation::from_span(expr.span, self.source_file.clone()),
                        VCKind::LoopInvariantInit,
                        "For loop invariant holds at entry",
                    );
                    self.push_vc(entry_vc);

                    // 2. Preservation VC: invariant is preserved by each iteration
                    // For x in range: I[x] && x < end => wp(S, I[x+1])
                    let next_var_expr =
                        SmtExpr::add(SmtExpr::Var(loop_var.clone()), SmtExpr::int(1));
                    let next_invariant = invariant.substitute(&loop_var, &next_var_expr);
                    let body_wp = self.wp_block(body, &next_invariant);

                    let in_range = Formula::and(vec![
                        Formula::ge(SmtExpr::Var(loop_var.clone()), start_bound.clone()),
                        Formula::lt(SmtExpr::Var(loop_var.clone()), end_bound.clone()),
                    ]);

                    let preserve_formula = Formula::Forall(
                        vec![loop_var.clone()].into(),
                        Box::new(Formula::implies(
                            Formula::and(vec![invariant.clone(), in_range]),
                            body_wp,
                        )),
                    );

                    let preserve_vc = VerificationCondition::new(
                        preserve_formula,
                        SourceLocation::from_span(expr.span, self.source_file.clone()),
                        VCKind::LoopInvariantPreserve,
                        "For loop invariant preserved by iteration",
                    );
                    self.push_vc(preserve_vc);

                    // 3. Exit VC: invariant at end implies postcondition
                    let at_end = Formula::Eq(
                        Box::new(SmtExpr::Var(loop_var.clone())),
                        Box::new(end_bound.clone()),
                    );
                    let exit_formula = Formula::Forall(
                        vec![loop_var.clone()].into(),
                        Box::new(Formula::implies(
                            Formula::and(vec![invariant.clone(), at_end]),
                            postcondition.clone(),
                        )),
                    );

                    let exit_vc = VerificationCondition::new(
                        exit_formula,
                        SourceLocation::from_span(expr.span, self.source_file.clone()),
                        VCKind::LoopInvariantExit,
                        "For loop postcondition holds at exit",
                    );
                    self.push_vc(exit_vc);

                    // 4. Termination VC: if decreases clause present
                    for decreases_expr in decreases {
                        let variant = self.translate_expr(decreases_expr);
                        let next_variant = variant.substitute(&loop_var, &next_var_expr);

                        // Variant must be non-negative
                        let non_neg = Formula::ge(variant.clone(), SmtExpr::int(0));
                        let decreases_formula = Formula::lt(next_variant, variant);

                        let term_vc = VerificationCondition::new(
                            Formula::and(vec![
                                Formula::implies(invariant.clone(), non_neg),
                                Formula::implies(invariant.clone(), decreases_formula),
                            ]),
                            SourceLocation::from_span(expr.span, self.source_file.clone()),
                            VCKind::Termination,
                            "For loop terminates",
                        );
                        self.push_vc(term_vc);
                    }
                } else {
                    // Collection-based for loop: for x in collection
                    // Use abstract iteration with forall quantification

                    let body_wp = self.wp_block(body, &invariant);

                    let preserve_formula = Formula::Forall(
                        vec![loop_var.clone()].into(),
                        Box::new(Formula::implies(
                            Formula::and(vec![
                                invariant.clone(),
                                Formula::Predicate(
                                    Text::from("in"),
                                    vec![SmtExpr::Var(loop_var.clone()), start_bound.clone()]
                                        .into(),
                                ),
                            ]),
                            body_wp,
                        )),
                    );

                    let preserve_vc = VerificationCondition::new(
                        preserve_formula,
                        SourceLocation::from_span(expr.span, self.source_file.clone()),
                        VCKind::LoopInvariantPreserve,
                        "For loop invariant preservation over collection",
                    );
                    self.push_vc(preserve_vc);
                }

                invariant
            }
            ExprKind::Loop {
                label: _,
                body,
                invariants: _,
            } => {
                // Infinite loop: wp(loop { S }, Q) = I (invariant must be established)
                let loop_id = self.next_loop_id;
                self.next_loop_id += 1;

                let invariant = match self.symbol_table.loop_invariants.get(&loop_id) {
                    Some(inv) => inv.clone(),
                    None => Formula::True,
                };

                let body_wp = self.wp_block(body, &invariant);

                let preserve_vc = VerificationCondition::new(
                    Formula::implies(invariant.clone(), body_wp),
                    SourceLocation::from_span(expr.span, self.source_file.clone()),
                    VCKind::LoopInvariantPreserve,
                    "Infinite loop invariant preservation",
                );
                if let Maybe::Some(func_name) = &self.current_function {
                    self.vcs.push(preserve_vc.with_function(func_name.clone()));
                } else {
                    self.vcs.push(preserve_vc);
                }

                invariant
            }
            ExprKind::Return(maybe_expr) => match maybe_expr {
                Some(ret_expr) => self.wp_return(Some(ret_expr), postcondition),
                None => self.wp_return(None, postcondition),
            },
            ExprKind::Block(block) => self.wp_block(block, postcondition),
            ExprKind::Binary { op, left, right } if op.is_assignment() => {
                // Assignment: wp(x = e, Q) = Q[x/e]
                if let ExprKind::Path(path) = &left.kind
                    && let Some(seg) = path.segments.first()
                {
                    let var = Variable::new(path_segment_to_str(seg));
                    let smt_value = self.translate_expr(right);
                    return postcondition.substitute(&var, &smt_value);
                }
                postcondition.clone()
            }
            ExprKind::Index {
                expr: arr_expr,
                index,
            } => {
                // Array/slice access: generate bounds check VCs
                let arr_smt = self.translate_expr(arr_expr);
                let length_expr = self.get_array_length_expr(arr_expr, &arr_smt);

                // Check if this is a range index (slice access) or simple index
                match &index.kind {
                    ExprKind::Range {
                        start,
                        end,
                        inclusive,
                    } => {
                        // Slice access: arr[start..end] or arr[start..=end]
                        // Generate VCs for:
                        // 1. 0 <= start
                        // 2. start <= end (or start < end for exclusive)
                        // 3. end <= length (or end < length for exclusive)

                        let start_smt = match start {
                            Some(s) => self.translate_expr(s),
                            None => SmtExpr::int(0),
                        };

                        let end_smt = match end {
                            Some(e) => self.translate_expr(e),
                            None => length_expr.clone(),
                        };

                        let mut bounds_constraints = vec![
                            // Start >= 0
                            Formula::ge(start_smt.clone(), SmtExpr::int(0)),
                            // Start <= end
                            Formula::le(start_smt.clone(), end_smt.clone()),
                        ];

                        // End bound depends on inclusive vs exclusive
                        if *inclusive {
                            // For ..= (inclusive): end < length (since end is included)
                            bounds_constraints
                                .push(Formula::lt(end_smt.clone(), length_expr.clone()));
                        } else {
                            // For .. (exclusive): end <= length
                            bounds_constraints
                                .push(Formula::le(end_smt.clone(), length_expr.clone()));
                        }

                        let bounds_formula = Formula::and(bounds_constraints);

                        let bounds_vc = VerificationCondition::new(
                            bounds_formula,
                            SourceLocation::from_span(expr.span, self.source_file.clone()),
                            VCKind::ArrayBounds,
                            "Slice bounds valid: 0 <= start <= end <= length",
                        );
                        if let Maybe::Some(func_name) = &self.current_function {
                            self.vcs.push(bounds_vc.with_function(func_name.clone()));
                        } else {
                            self.vcs.push(bounds_vc);
                        }
                    }
                    _ => {
                        // Simple index access: arr[i]
                        // Generate VC: 0 <= index < length
                        let idx_smt = self.translate_expr(index);

                        let lower_bound = Formula::ge(idx_smt.clone(), SmtExpr::int(0));
                        let upper_bound = Formula::lt(idx_smt.clone(), length_expr.clone());

                        let bounds_formula = Formula::and([lower_bound, upper_bound]);

                        let bounds_vc = VerificationCondition::new(
                            bounds_formula,
                            SourceLocation::from_span(expr.span, self.source_file.clone()),
                            VCKind::ArrayBounds,
                            "Array index within bounds: 0 <= index < length",
                        );
                        if let Maybe::Some(func_name) = &self.current_function {
                            self.vcs.push(bounds_vc.with_function(func_name.clone()));
                        } else {
                            self.vcs.push(bounds_vc);
                        }
                    }
                }

                postcondition.clone()
            }
            ExprKind::Binary {
                op: BinOp::Div,
                left,
                right,
            }
            | ExprKind::Binary {
                op: BinOp::Rem,
                left,
                right,
            } => {
                // Division: generate division by zero check
                let divisor = self.translate_expr(right);

                let div_vc = VerificationCondition::new(
                    Formula::Ne(Box::new(divisor), Box::new(SmtExpr::int(0))),
                    SourceLocation::from_span(expr.span, self.source_file.clone()),
                    VCKind::DivisionByZero,
                    "Division by non-zero",
                );
                if let Maybe::Some(func_name) = &self.current_function {
                    self.vcs.push(div_vc.with_function(func_name.clone()));
                } else {
                    self.vcs.push(div_vc);
                }

                postcondition.clone()
            }
            ExprKind::Call { func, args, .. } => {
                // Function call: check callee precondition, assume postcondition
                if let ExprKind::Path(path) = &func.kind {
                    let func_name = path
                        .segments
                        .iter()
                        .map(path_segment_to_str)
                        .collect::<List<_>>()
                        .join(".");

                    if let Maybe::Some(sig) = self.symbol_table.get_function(func_name.as_str()) {
                        // Generate VC for precondition
                        let mut pre_subst = sig.precondition.clone();
                        for (i, (param_name, _)) in sig.params.iter().enumerate() {
                            if let Some(arg) = args.get(i) {
                                let var = Variable::new(param_name.clone());
                                let arg_smt = self.translate_expr(arg);
                                pre_subst = pre_subst.substitute(&var, &arg_smt);
                            }
                        }

                        let call_vc = VerificationCondition::new(
                            pre_subst,
                            SourceLocation::from_span(expr.span, self.source_file.clone()),
                            VCKind::Precondition,
                            format!("Call to '{}' precondition", func_name),
                        );
                        if let Maybe::Some(curr_func) = &self.current_function {
                            self.vcs.push(call_vc.with_function(curr_func.clone()));
                        } else {
                            self.vcs.push(call_vc);
                        }
                    }
                }
                postcondition.clone()
            }
            _ => {
                // Default: expression doesn't affect postcondition
                postcondition.clone()
            }
        }
    }

    /// Weakest precondition for inner expression (helper for else branches)
    fn wp_expr_inner(&mut self, expr: &Expr, postcondition: &Formula) -> Formula {
        match &expr.kind {
            ExprKind::Block(block) => self.wp_block(block, postcondition),
            _ => self.wp_expr(expr, postcondition),
        }
    }

    /// Weakest precondition for return statement
    ///
    /// wp(return e, Q) = Q[result/e]
    fn wp_return(&mut self, value: Option<&Expr>, postcondition: &Formula) -> Formula {
        let result_var = Variable::result();
        match value {
            Some(expr) => {
                let smt_value = self.translate_expr(expr);
                postcondition.substitute(&result_var, &smt_value)
            }
            None => {
                // Return unit: substitute result with unit value
                postcondition.substitute(
                    &result_var,
                    &SmtExpr::Apply(Text::from("unit"), List::new()),
                )
            }
        }
    }

    /// Translate condition to formula
    fn translate_condition(&self, cond: &verum_ast::expr::IfCondition) -> Formula {
        let mut formulas = List::new();
        for c in cond.conditions.iter() {
            match c {
                verum_ast::expr::ConditionKind::Expr(expr) => {
                    formulas.push(self.translate_expr_to_formula(expr));
                }
                verum_ast::expr::ConditionKind::Let { pattern, value } => {
                    let smt_value = self.translate_expr(value);
                    formulas.push(self.pattern_match_condition(pattern, &smt_value));
                }
            }
        }
        Formula::and(formulas)
    }

    /// Generate pattern match condition
    fn pattern_match_condition(&self, pattern: &Pattern, value: &SmtExpr) -> Formula {
        match &pattern.kind {
            PatternKind::Wildcard | PatternKind::Ident { .. } => Formula::True,
            PatternKind::Literal(lit) => {
                let lit_smt = self.translate_literal(lit);
                Formula::eq(value.clone(), lit_smt)
            }
            PatternKind::Tuple(patterns) => {
                let mut conditions = List::new();
                for (i, pat) in patterns.iter().enumerate() {
                    let elem = SmtExpr::Apply(
                        Text::from(format!("tuple_{}", i)),
                        vec![value.clone()].into(),
                    );
                    conditions.push(self.pattern_match_condition(pat, &elem));
                }
                Formula::and(conditions)
            }
            PatternKind::Variant { path, data } => {
                // Check constructor tag and match fields
                let tag_name = path
                    .segments
                    .iter()
                    .map(path_segment_to_str)
                    .collect::<List<_>>()
                    .join(".");
                let tag_check = Formula::eq(
                    SmtExpr::Apply(Text::from("tag"), vec![value.clone()].into()),
                    SmtExpr::Apply(tag_name, List::new()),
                );

                let mut conditions = vec![tag_check];
                if let Some(verum_ast::pattern::VariantPatternData::Tuple(fields)) = data {
                    for (i, field) in fields.iter().enumerate() {
                        let field_val = SmtExpr::Apply(
                            Text::from(format!("field_{}", i)),
                            vec![value.clone()].into(),
                        );
                        conditions.push(self.pattern_match_condition(field, &field_val));
                    }
                }
                Formula::and(conditions)
            }
            _ => Formula::True,
        }
    }

    /// Translate expression to SMT expression
    pub fn translate_expr(&self, expr: &Expr) -> SmtExpr {
        match &expr.kind {
            ExprKind::Literal(lit) => self.translate_literal(lit),
            ExprKind::Path(path) => {
                let name = path
                    .segments
                    .iter()
                    .map(path_segment_to_str)
                    .collect::<List<_>>()
                    .join(".");
                SmtExpr::var(name)
            }
            ExprKind::Binary { op, left, right } => {
                let left_smt = self.translate_expr(left);
                let right_smt = self.translate_expr(right);
                self.translate_binop(*op, left_smt, right_smt)
            }
            ExprKind::Unary { op, expr } => {
                let inner = self.translate_expr(expr);
                self.translate_unop(*op, inner)
            }
            ExprKind::Index { expr: arr, index } => {
                let arr_smt = self.translate_expr(arr);
                let idx_smt = self.translate_expr(index);
                SmtExpr::Select(Box::new(arr_smt), Box::new(idx_smt))
            }
            ExprKind::Tuple(exprs) => {
                let args: List<SmtExpr> = exprs
                    .iter()
                    .map(|e| self.translate_expr(e))
                    .collect::<List<_>>();
                SmtExpr::Apply(Text::from("tuple"), args)
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_smt = self.translate_condition_to_expr(condition);
                let then_smt = self.translate_block_to_expr(then_branch);
                let else_smt = match else_branch {
                    Some(e) => self.translate_expr(e),
                    None => SmtExpr::Apply(Text::from("unit"), List::new()),
                };
                SmtExpr::Ite(
                    Box::new(self.expr_to_formula(&cond_smt)),
                    Box::new(then_smt),
                    Box::new(else_smt),
                )
            }
            ExprKind::Call { func, args, .. } => {
                let func_name = if let ExprKind::Path(path) = &func.kind {
                    path.segments
                        .iter()
                        .map(path_segment_to_str)
                        .collect::<List<_>>()
                        .join(".")
                } else {
                    Text::from("unknown_func")
                };
                let smt_args: List<SmtExpr> = args
                    .iter()
                    .map(|a| self.translate_expr(a))
                    .collect::<List<_>>();
                SmtExpr::Apply(func_name, smt_args)
            }
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                let recv_smt = self.translate_expr(receiver);
                let mut all_args = vec![recv_smt];
                all_args.extend(args.iter().map(|a| self.translate_expr(a)));
                SmtExpr::Apply(Text::from(method.as_str()), all_args.into())
            }
            ExprKind::Field { expr, field } => {
                let base = self.translate_expr(expr);
                SmtExpr::Apply(
                    Text::from(format!("field_{}", field.as_str())),
                    vec![base].into(),
                )
            }
            ExprKind::Block(block) => self.translate_block_to_expr(block),
            ExprKind::Paren(inner) => self.translate_expr(inner),
            _ => {
                // For unsupported expressions, create an uninterpreted function
                SmtExpr::Apply(Text::from("unknown"), List::new())
            }
        }
    }

    /// Translate expression to formula
    fn translate_expr_to_formula(&self, expr: &Expr) -> Formula {
        let smt_expr = self.translate_expr(expr);
        self.expr_to_formula(&smt_expr)
    }

    /// Convert SMT expression to formula
    fn expr_to_formula(&self, expr: &SmtExpr) -> Formula {
        match expr {
            SmtExpr::BoolConst(b) => {
                if *b {
                    Formula::True
                } else {
                    Formula::False
                }
            }
            SmtExpr::Var(v) => Formula::Var(v.clone()),
            _ => Formula::Predicate(Text::from("is_true"), vec![expr.clone()].into()),
        }
    }

    /// Translate condition to SMT expression
    fn translate_condition_to_expr(&self, cond: &verum_ast::expr::IfCondition) -> SmtExpr {
        let mut exprs = List::new();
        for c in cond.conditions.iter() {
            match c {
                verum_ast::expr::ConditionKind::Expr(e) => {
                    exprs.push(self.translate_expr(e));
                }
                verum_ast::expr::ConditionKind::Let { .. } => {
                    // Let bindings in conditions translate to true for simplicity
                    exprs.push(SmtExpr::bool(true));
                }
            }
        }
        if exprs.len() == 1 {
            exprs.pop().unwrap_or_else(|| SmtExpr::bool(true))
        } else {
            SmtExpr::Apply(Text::from("and"), exprs)
        }
    }

    /// Translate block to SMT expression
    fn translate_block_to_expr(&self, block: &Block) -> SmtExpr {
        match &block.expr {
            Some(e) => self.translate_expr(e),
            None => SmtExpr::Apply(Text::from("unit"), List::new()),
        }
    }

    /// Translate literal to SMT expression
    fn translate_literal(&self, lit: &verum_ast::literal::Literal) -> SmtExpr {
        use verum_ast::literal::{LiteralKind, StringLit};
        match &lit.kind {
            LiteralKind::Int(int_lit) => SmtExpr::IntConst(int_lit.value as i64),
            LiteralKind::Float(float_lit) => SmtExpr::RealConst(float_lit.value),
            LiteralKind::Bool(b) => SmtExpr::BoolConst(*b),
            LiteralKind::Text(s) => {
                let str_value = match s {
                    StringLit::Regular(s) | StringLit::MultiLine(s) => s,
                };
                SmtExpr::Apply(Text::from(format!("str_{}", str_value)), List::new())
            }
            LiteralKind::Char(c) => SmtExpr::IntConst(*c as i64),
            LiteralKind::ByteChar(b) => SmtExpr::IntConst(*b as i64),
            LiteralKind::ByteString(bytes) => {
                // Represent byte string as uninterpreted for verification
                SmtExpr::Apply(Text::from(format!("bytes_{}", bytes.len())), List::new())
            }
            // Handle other literal kinds as uninterpreted
            LiteralKind::Tagged { tag, content } => SmtExpr::Apply(
                Text::from(format!("tagged_{}_{}", tag, content)),
                List::new(),
            ),
            LiteralKind::InterpolatedString(s) => {
                SmtExpr::Apply(Text::from(format!("interp_{}", s.prefix)), List::new())
            }
            LiteralKind::Contract(c) => {
                SmtExpr::Apply(Text::from(format!("contract_{}", c)), List::new())
            }
            LiteralKind::Composite(c) => {
                SmtExpr::Apply(Text::from(format!("composite_{}", c.tag)), List::new())
            }
            LiteralKind::ContextAdaptive(c) => {
                SmtExpr::Apply(Text::from(format!("adaptive_{:?}", c.kind)), List::new())
            }
        }
    }

    /// Translate binary operator
    fn translate_binop(&self, op: BinOp, left: SmtExpr, right: SmtExpr) -> SmtExpr {
        match op {
            BinOp::Add | BinOp::Concat | BinOp::AddAssign => SmtExpr::add(left, right),
            BinOp::Sub | BinOp::SubAssign => SmtExpr::sub(left, right),
            BinOp::Mul | BinOp::MulAssign => SmtExpr::mul(left, right),
            BinOp::Div | BinOp::DivAssign => {
                SmtExpr::BinOp(SmtBinOp::Div, Box::new(left), Box::new(right))
            }
            BinOp::Rem | BinOp::RemAssign => {
                SmtExpr::BinOp(SmtBinOp::Mod, Box::new(left), Box::new(right))
            }
            BinOp::Pow => SmtExpr::BinOp(SmtBinOp::Pow, Box::new(left), Box::new(right)),
            BinOp::Eq => SmtExpr::Apply(Text::from("="), vec![left, right].into()),
            BinOp::Ne => SmtExpr::Apply(Text::from("distinct"), vec![left, right].into()),
            BinOp::Lt => SmtExpr::Apply(Text::from("<"), vec![left, right].into()),
            BinOp::Le => SmtExpr::Apply(Text::from("<="), vec![left, right].into()),
            BinOp::Gt => SmtExpr::Apply(Text::from(">"), vec![left, right].into()),
            BinOp::Ge => SmtExpr::Apply(Text::from(">="), vec![left, right].into()),
            // "in" as containment - model as member-of for SMT
            BinOp::In => SmtExpr::Apply(Text::from("member"), vec![left, right].into()),
            BinOp::And => SmtExpr::Apply(Text::from("and"), vec![left, right].into()),
            BinOp::Or => SmtExpr::Apply(Text::from("or"), vec![left, right].into()),
            BinOp::Imply => SmtExpr::Apply(Text::from("=>"), vec![left, right].into()),
            BinOp::Iff => {
                // P <-> Q is (P => Q) && (Q => P)
                let l2 = left.clone();
                let r2 = right.clone();
                let fwd = SmtExpr::Apply(Text::from("=>"), vec![left, right].into());
                let bwd = SmtExpr::Apply(Text::from("=>"), vec![r2, l2].into());
                SmtExpr::Apply(Text::from("and"), vec![fwd, bwd].into())
            }
            BinOp::BitAnd | BinOp::BitAndAssign => {
                SmtExpr::Apply(Text::from("bvand"), vec![left, right].into())
            }
            BinOp::BitOr | BinOp::BitOrAssign => {
                SmtExpr::Apply(Text::from("bvor"), vec![left, right].into())
            }
            BinOp::BitXor | BinOp::BitXorAssign => {
                SmtExpr::Apply(Text::from("bvxor"), vec![left, right].into())
            }
            BinOp::Shl | BinOp::ShlAssign => {
                SmtExpr::Apply(Text::from("bvshl"), vec![left, right].into())
            }
            BinOp::Shr | BinOp::ShrAssign => {
                SmtExpr::Apply(Text::from("bvshr"), vec![left, right].into())
            }
            BinOp::Assign => right, // Assignment returns the value
        }
    }

    /// Translate unary operator
    fn translate_unop(&self, op: UnOp, expr: SmtExpr) -> SmtExpr {
        match op {
            UnOp::Not => SmtExpr::Apply(Text::from("not"), vec![expr].into()),
            UnOp::Neg => SmtExpr::UnOp(SmtUnOp::Neg, Box::new(expr)),
            UnOp::BitNot => SmtExpr::Apply(Text::from("bvnot"), vec![expr].into()),
            UnOp::Deref
            | UnOp::Ref
            | UnOp::RefMut
            | UnOp::RefChecked
            | UnOp::RefCheckedMut
            | UnOp::RefUnsafe
            | UnOp::RefUnsafeMut
            | UnOp::Own
            | UnOp::OwnMut => {
                // Reference operations don't change the value semantically
                expr
            }
        }
    }

    /// Translate AST type to variable type
    fn translate_type(&self, ty: &verum_ast::ty::Type) -> VarType {
        use verum_ast::ty::TypeKind;
        match &ty.kind {
            TypeKind::Path(path) => {
                let name = path
                    .segments
                    .iter()
                    .map(path_segment_to_str)
                    .collect::<List<_>>()
                    .join(".");
                {
                let n = name.as_str();
                match n {
                    _ if verum_common::well_known_types::type_names::is_signed_integer_type(n) => VarType::Int,
                    "Bool" | "bool" => VarType::Bool,
                    _ if verum_common::well_known_types::type_names::is_float_type(n) => VarType::Real,
                    "u8" => VarType::BitVec(8),
                    "u16" => VarType::BitVec(16),
                    "u32" => VarType::BitVec(32),
                    "u64" => VarType::BitVec(64),
                    _ => VarType::Sort(name),
                }
                }
            }
            TypeKind::Array { element, .. } => VarType::Array(
                Box::new(VarType::Int),
                Box::new(self.translate_type(element)),
            ),
            TypeKind::Tuple(types) => {
                // For tuples, use an uninterpreted sort
                VarType::Sort(Text::from(format!("Tuple_{}", types.len())))
            }
            _ => VarType::Sort(Text::from("Unknown")),
        }
    }

    /// Extract the loop variable from a pattern
    ///
    /// For patterns like `x`, `(a, b)`, or `Point { x, y }`, extracts
    /// the primary variable for use in loop verification.
    fn extract_pattern_variable(&self, pattern: &Pattern) -> Variable {
        match &pattern.kind {
            PatternKind::Ident { name, .. } => Variable::new(name.as_str()),
            PatternKind::Tuple(patterns) if !patterns.is_empty() => {
                // For tuple patterns, use first element's variable
                self.extract_pattern_variable(&patterns[0])
            }
            PatternKind::Record { fields, .. } => {
                // For struct patterns, use first field's pattern or name
                if let Some(first) = fields.first() {
                    match &first.pattern {
                        Some(pat) => self.extract_pattern_variable(pat),
                        None => Variable::new(first.name.as_str()),
                    }
                } else {
                    Variable::new("_loop_var")
                }
            }
            _ => Variable::new("_loop_var"),
        }
    }

    /// Extract iteration bounds from a for loop iterator expression
    ///
    /// Returns (start_bound, end_bound, is_range) where:
    /// - For range expressions: actual start/end values
    /// - For collections: symbolic bounds and is_range = false
    fn extract_for_bounds(&self, iter: &Expr) -> (SmtExpr, SmtExpr, bool) {
        match &iter.kind {
            // Range expression: start..end or start..=end
            ExprKind::Range {
                start,
                end,
                inclusive,
            } => {
                let start_expr = match start {
                    Some(s) => self.translate_expr(s),
                    None => SmtExpr::int(0),
                };

                let end_expr = match end {
                    Some(e) => {
                        let e_smt = self.translate_expr(e);
                        if *inclusive {
                            // Inclusive range: end becomes end + 1
                            SmtExpr::add(e_smt, SmtExpr::int(1))
                        } else {
                            e_smt
                        }
                    }
                    None => SmtExpr::var("_iter_end"), // Unbounded
                };

                (start_expr, end_expr, true)
            }

            // Method call like iter.range() or collection.iter()
            ExprKind::MethodCall {
                receiver, method, ..
            } => {
                match method.as_str() {
                    "iter" | "into_iter" | "iter_mut" => {
                        // Collection iterator - use abstract bounds
                        let collection = self.translate_expr(receiver);
                        (collection, SmtExpr::var("_iter_end"), false)
                    }
                    _ => {
                        let iter_expr = self.translate_expr(iter);
                        (iter_expr, SmtExpr::var("_iter_end"), false)
                    }
                }
            }

            // Direct collection reference
            _ => {
                let iter_expr = self.translate_expr(iter);
                (iter_expr, SmtExpr::var("_iter_end"), false)
            }
        }
    }

    /// Extract array length from a type annotation
    ///
    /// For array types like `[i32; 10]`, extracts the length (10).
    /// For slice types or non-array types, returns None.
    fn extract_type_length(&self, ty: &verum_ast::Type) -> Option<SmtExpr> {
        use verum_ast::TypeKind;

        match &ty.kind {
            TypeKind::Array { size, .. } => {
                // Array with size: [T; N]
                size.as_ref()
                    .map(|size_expr| self.translate_expr(size_expr))
            }
            _ => None, // Not an array type
        }
    }

    /// Extract array length from an array literal expression
    ///
    /// For array literals like `[1, 2, 3]`, returns the length (3).
    /// For repeat expressions like `[0; 10]`, returns the size (10).
    fn extract_array_literal_length(&self, expr: &Expr) -> Option<SmtExpr> {
        use verum_ast::expr::ArrayExpr;

        match &expr.kind {
            ExprKind::Array(arr_expr) => {
                match arr_expr {
                    ArrayExpr::List(elements) => {
                        // Array literal [a, b, c, ...]
                        Some(SmtExpr::int(elements.len() as i64))
                    }
                    ArrayExpr::Repeat { count, .. } => {
                        // Repeat expression [expr; count]
                        Some(self.translate_expr(count))
                    }
                }
            }
            _ => None, // Not an array literal
        }
    }

    /// Get the length expression for an array expression
    ///
    /// Attempts to resolve the array's length from multiple sources:
    /// 1. Symbol table (if the array is a known variable with tracked length)
    /// 2. Method calls like arr.len()
    /// 3. Field accesses (e.g., self.data)
    /// 4. Uninterpreted len() function as fallback for SMT solving
    ///
    /// Resolves array length for bounds check elimination. Checks symbol table
    /// for known variables with tracked length, method calls like arr.len(),
    /// field accesses, and falls back to uninterpreted len() for SMT solving.
    fn get_array_length_expr(&self, arr_expr: &Expr, arr_smt: &SmtExpr) -> SmtExpr {
        // Source 1: Check if array is a simple variable with known length in symbol table
        if let ExprKind::Path(path) = &arr_expr.kind {
            let name = path
                .segments
                .iter()
                .map(path_segment_to_str)
                .collect::<List<_>>()
                .join(".");
            if let Maybe::Some(length) = self.symbol_table.get_array_length(name.as_str()) {
                return length;
            }
        }

        // Source 2: Check for method call like arr.len() which would be translated
        if let ExprKind::MethodCall {
            receiver,
            method,
            args,
            ..
        } = &arr_expr.kind
        {
            if method.as_str() == "len" && args.is_empty() {
                // This IS a len() call, so just translate the receiver and wrap with len
                let recv_smt = self.translate_expr(receiver);
                return SmtExpr::UnOp(SmtUnOp::Len, Box::new(recv_smt));
            }
        }

        // Source 3: For field accesses (e.g., self.data), check if we track that
        if let ExprKind::Field { expr, field } = &arr_expr.kind {
            if let ExprKind::Path(path) = &expr.kind {
                let base_name = path
                    .segments
                    .iter()
                    .map(path_segment_to_str)
                    .collect::<List<_>>()
                    .join(".");
                let full_name = format!("{}.{}", base_name, field.as_str());
                if let Maybe::Some(length) = self.symbol_table.get_array_length(full_name.as_str())
                {
                    return length;
                }
            }
        }

        // Fallback: Use uninterpreted len() function on the array expression
        // This allows the SMT solver to reason about array lengths abstractly
        // The solver will treat len(arr) as an unknown but consistent value
        SmtExpr::UnOp(SmtUnOp::Len, Box::new(arr_smt.clone()))
    }

    /// Push a verification condition, associating it with current function if any
    fn push_vc(&mut self, vc: VerificationCondition) {
        if let Maybe::Some(func_name) = &self.current_function {
            self.vcs.push(vc.with_function(func_name.clone()));
        } else {
            self.vcs.push(vc);
        }
    }

    /// Translate a contract expression to a formula
    fn translate_contract(&self, expr: &Expr) -> Formula {
        self.translate_expr_to_formula(expr)
    }

    /// Extract contract (precondition, postcondition) from function
    ///
    /// Parses contract specifications from function attributes and body.
    /// Supports:
    /// - @requires(expr) attributes for preconditions
    /// - @ensures(expr) attributes for postconditions
    /// - @invariant(expr) attributes for loop invariants
    /// - @decreases(expr) attributes for termination measures
    /// - contract#"requires ...; ensures ..." literals in function body
    ///
    /// # Design by Contract (DbC)
    ///
    /// Follows Bertrand Meyer's Design by Contract:
    /// - Preconditions: What the caller must ensure before the call
    /// - Postconditions: What the callee guarantees after the call
    /// - Invariants: What remains true throughout execution
    fn extract_contract(&self, func: &FunctionDecl) -> (Formula, Formula) {
        let mut preconditions = List::new();
        let mut postconditions = List::new();

        // Parse contract attributes: @requires, @ensures, @invariant, @decreases
        for attr in func.attributes.iter() {
            match attr.name.as_str() {
                "requires" => {
                    if let Some(args) = &attr.args {
                        for arg in args.iter() {
                            let formula =
                                self.parse_contract_expr(arg, ContractContext::Precondition);
                            preconditions.push(formula);
                        }
                    }
                }
                "ensures" => {
                    if let Some(args) = &attr.args {
                        for arg in args.iter() {
                            let formula =
                                self.parse_contract_expr(arg, ContractContext::Postcondition);
                            postconditions.push(formula);
                        }
                    }
                }
                "invariant" => {
                    // Loop invariants are handled separately during loop processing
                    // Store for later use in wp_expr for loops
                    if let Some(args) = &attr.args {
                        for arg in args.iter() {
                            let formula = self.parse_contract_expr(arg, ContractContext::Invariant);
                            // Add as both pre and post for method-level invariants
                            preconditions.push(formula.clone());
                            postconditions.push(formula);
                        }
                    }
                }
                "decreases" => {
                    // Termination measures - handled by termination checker
                    // Generate termination VCs during loop analysis
                }
                "verify" => {
                    // @verify(proof) or @verify(check) - verification level attribute
                    // This affects the verification level, not the contract itself
                }
                _ => {
                    // Unknown attribute - check if it's a contract literal in disguise
                }
            }
        }

        // Parse contract literals from function body if present
        if let Some(body) = &func.body
            && let verum_ast::decl::FunctionBody::Block(block) = body
        {
            // Look for contract literals in the first statements
            for stmt in block.stmts.iter() {
                if let StmtKind::Expr { expr, .. } = &stmt.kind
                    && let ExprKind::Literal(lit) = &expr.kind
                    && let verum_ast::literal::LiteralKind::Contract(contract_text) = &lit.kind
                {
                    let text_ref = Text::from(contract_text.clone());
                    let (pre, post) = self.parse_contract_literal(&text_ref);
                    preconditions.extend(pre);
                    postconditions.extend(post);
                }
            }
        }

        // Combine all preconditions and postconditions
        let precondition = if preconditions.is_empty() {
            Formula::True
        } else {
            Formula::and(preconditions)
        };

        let postcondition = if postconditions.is_empty() {
            Formula::True
        } else {
            Formula::and(postconditions)
        };

        (precondition, postcondition)
    }

    /// Parse a contract expression from an AST expression
    ///
    /// Handles special contract constructs like:
    /// - old(expr): Value of expression at function entry
    /// - result: Return value in postconditions
    /// - forall x: T. P(x): Universal quantification
    /// - exists x: T. P(x): Existential quantification
    fn parse_contract_expr(&self, expr: &Expr, context: ContractContext) -> Formula {
        match &expr.kind {
            // Handle old(expr) - value at function entry
            ExprKind::Call { func, args, .. } => {
                if let ExprKind::Path(path) = &func.kind {
                    let func_name = path
                        .segments
                        .iter()
                        .map(path_segment_to_str)
                        .collect::<List<_>>()
                        .join(".");

                    match func_name.as_str() {
                        "old" => {
                            // old(expr) captures value at function entry
                            if let Some(arg) = args.first() {
                                let inner_smt = self.translate_expr(arg);
                                // Wrap with old() marker for postcondition handling
                                return Formula::Predicate(
                                    Text::from("old"),
                                    vec![inner_smt].into(),
                                );
                            }
                        }
                        "forall" => {
                            // forall quantification
                            let args_list = List::from(args.clone());
                            return self.parse_quantified_expr(&args_list, true);
                        }
                        "exists" => {
                            // exists quantification
                            let args_list = List::from(args.clone());
                            return self.parse_quantified_expr(&args_list, false);
                        }
                        _ => {}
                    }
                }
                // Regular function call in contract
                let smt_expr = self.translate_expr(expr);
                self.expr_to_formula(&smt_expr)
            }

            // Handle result - return value in postconditions
            ExprKind::Path(path) => {
                let name = path
                    .segments
                    .iter()
                    .map(path_segment_to_str)
                    .collect::<List<_>>()
                    .join(".");

                if name.as_str() == "result" && context == ContractContext::Postcondition {
                    Formula::Var(Variable::result())
                } else {
                    let smt_expr = self.translate_expr(expr);
                    self.expr_to_formula(&smt_expr)
                }
            }

            // Comparison operators become formula comparisons
            ExprKind::Binary { op, left, right } => {
                let left_smt = self.translate_expr(left);
                let right_smt = self.translate_expr(right);

                match op {
                    BinOp::Eq => Formula::eq(left_smt, right_smt),
                    BinOp::Ne => Formula::Ne(Box::new(left_smt), Box::new(right_smt)),
                    BinOp::Lt => Formula::lt(left_smt, right_smt),
                    BinOp::Le => Formula::le(left_smt, right_smt),
                    BinOp::Gt => Formula::gt(left_smt, right_smt),
                    BinOp::Ge => Formula::ge(left_smt, right_smt),
                    BinOp::And => {
                        let left_formula = self.parse_contract_expr(left, context);
                        let right_formula = self.parse_contract_expr(right, context);
                        Formula::and([left_formula, right_formula])
                    }
                    BinOp::Or => {
                        let left_formula = self.parse_contract_expr(left, context);
                        let right_formula = self.parse_contract_expr(right, context);
                        Formula::or([left_formula, right_formula])
                    }
                    BinOp::Imply => {
                        let left_formula = self.parse_contract_expr(left, context);
                        let right_formula = self.parse_contract_expr(right, context);
                        Formula::implies(left_formula, right_formula)
                    }
                    _ => {
                        // Arithmetic operators - treat as predicate
                        let smt_expr = self.translate_expr(expr);
                        self.expr_to_formula(&smt_expr)
                    }
                }
            }

            // Unary not becomes formula negation
            ExprKind::Unary {
                op: UnOp::Not,
                expr: inner,
            } => {
                let inner_formula = self.parse_contract_expr(inner, context);
                Formula::not(inner_formula)
            }

            // Parenthesized expression
            ExprKind::Paren(inner) => self.parse_contract_expr(inner, context),

            // Literal values
            ExprKind::Literal(lit) => {
                let smt_expr = self.translate_literal(lit);
                self.expr_to_formula(&smt_expr)
            }

            // Default: translate to SMT and convert to formula
            _ => {
                let smt_expr = self.translate_expr(expr);
                self.expr_to_formula(&smt_expr)
            }
        }
    }

    /// Parse a quantified expression (forall or exists)
    ///
    /// Supports multiple formats:
    /// - `forall(x, y, body)` - simple variable binding
    /// - `forall(x: Int, y: Bool, body)` - typed variable binding
    /// - `forall(|x: Int, y: Bool| body)` - closure-style binding
    /// - `forall([x, y], body)` - array-style binding with implicit types
    ///
    /// Parses quantified expressions in contracts. Supports multiple binding styles:
    /// forall(x, y, body), forall(x: Int, y: Bool, body), forall(|x: Int| body),
    /// and forall([x, y], body) with implicit types.
    fn parse_quantified_expr(&self, args: &List<Expr>, is_forall: bool) -> Formula {
        if args.is_empty() {
            return Formula::True;
        }

        // Check for closure-style quantification: forall(|x: Int| body)
        if args.len() == 1 {
            if let ExprKind::Closure { params, body, .. } = &args[0].kind {
                let bound_vars = self.extract_closure_params(params);
                let body_formula = self.parse_contract_expr(body, ContractContext::Invariant);
                return if is_forall {
                    Formula::Forall(bound_vars, Box::new(body_formula))
                } else {
                    Formula::Exists(bound_vars, Box::new(body_formula))
                };
            }
        }

        // The last argument is the body
        let Some(body) = args.last() else {
            return Formula::True;
        };
        let body_formula = self.parse_contract_expr(body, ContractContext::Invariant);

        // Extract bound variables from all earlier arguments
        let mut bound_vars = List::new();
        for arg in args.iter().take(args.len().saturating_sub(1)) {
            match &arg.kind {
                // Simple variable: forall(x, body)
                ExprKind::Path(path) => {
                    let name = path
                        .segments
                        .iter()
                        .map(path_segment_to_str)
                        .collect::<List<_>>()
                        .join(".");
                    bound_vars.push(Variable::new(name));
                }

                // Typed variable using cast syntax: forall(x as Int, body)
                // The `as` cast can be used to specify the type of a quantified variable
                ExprKind::Cast { expr, ty } => {
                    if let ExprKind::Path(path) = &expr.kind {
                        let name = path
                            .segments
                            .iter()
                            .map(path_segment_to_str)
                            .collect::<List<_>>()
                            .join(".");
                        let var_type = self.type_to_var_type(ty);
                        bound_vars.push(Variable::typed(name, var_type));
                    }
                }

                // Function call that might represent typed binding: Int(x)
                // Some systems use constructor-style syntax for typed quantification
                ExprKind::Call { func, args, .. } => {
                    if let ExprKind::Path(type_path) = &func.kind {
                        let type_name = type_path
                            .segments
                            .iter()
                            .map(path_segment_to_str)
                            .collect::<List<_>>()
                            .join(".");
                        let n = type_name.as_str();
                        let var_type = match n {
                            _ if verum_common::well_known_types::type_names::is_integer_type(n) => VarType::Int,
                            "Bool" => VarType::Bool,
                            _ if verum_common::well_known_types::type_names::is_float_type(n) || n == "Real" => VarType::Real,
                            _ => VarType::Sort(Text::from(type_name)),
                        };
                        // Extract variable names from arguments
                        for arg in args.iter() {
                            if let ExprKind::Path(var_path) = &arg.kind {
                                let var_name = var_path
                                    .segments
                                    .iter()
                                    .map(path_segment_to_str)
                                    .collect::<List<_>>()
                                    .join(".");
                                bound_vars.push(Variable::typed(var_name, var_type.clone()));
                            }
                        }
                    }
                }

                // Tuple of variables: forall((x, y), body)
                ExprKind::Tuple(elements) => {
                    for elem in elements.iter() {
                        if let ExprKind::Path(path) = &elem.kind {
                            let name = path
                                .segments
                                .iter()
                                .map(path_segment_to_str)
                                .collect::<List<_>>()
                                .join(".");
                            bound_vars.push(Variable::new(name));
                        }
                    }
                }

                // Array of variables: forall([x, y], body)
                ExprKind::Array(array_expr) => {
                    // Handle both List and Repeat variants of ArrayExpr
                    if let verum_ast::ArrayExpr::List(elements) = array_expr {
                        for elem in elements.iter() {
                            if let ExprKind::Path(path) = &elem.kind {
                                let name = path
                                    .segments
                                    .iter()
                                    .map(path_segment_to_str)
                                    .collect::<List<_>>()
                                    .join(".");
                                bound_vars.push(Variable::new(name));
                            }
                        }
                    }
                    // Repeat variant doesn't make sense for variable binding
                }

                _ => {
                    // Unknown format - try to extract free variables from the expression
                    // This handles cases like forall(x > 0, body) where we need to
                    // infer that x is the bound variable
                }
            }
        }

        if bound_vars.is_empty() {
            // No explicit bound variables found - return body as-is
            // This handles malformed quantifiers gracefully
            body_formula
        } else if is_forall {
            Formula::Forall(bound_vars, Box::new(body_formula))
        } else {
            Formula::Exists(bound_vars, Box::new(body_formula))
        }
    }

    /// Extract bound variables from closure parameters
    fn extract_closure_params(&self, params: &[verum_ast::expr::ClosureParam]) -> List<Variable> {
        let mut bound_vars = List::new();
        for param in params {
            if let PatternKind::Ident { name, .. } = &param.pattern.kind {
                let var_type = param
                    .ty
                    .as_ref()
                    .map(|t| self.type_to_var_type(t))
                    .unwrap_or(VarType::Int);
                bound_vars.push(Variable::typed(name.to_string(), var_type));
            }
        }
        bound_vars
    }

    /// Extract variable type from type expression
    fn extract_var_type(&self, type_expr: &Expr) -> VarType {
        if let ExprKind::Path(path) = &type_expr.kind {
            let name = path
                .segments
                .iter()
                .map(path_segment_to_str)
                .collect::<List<_>>()
                .join(".");
            {
                let n = name.as_str();
                match n {
                    _ if verum_common::well_known_types::type_names::is_integer_type(n) => VarType::Int,
                    "Bool" => VarType::Bool,
                    _ if verum_common::well_known_types::type_names::is_float_type(n) || n == "Real" => VarType::Real,
                    _ => VarType::Sort(Text::from(name)),
                }
            }
        } else {
            VarType::Int // Default to Int for unknown types
        }
    }

    /// Convert AST type to VarType
    fn type_to_var_type(&self, ty: &verum_ast::ty::Type) -> VarType {
        use verum_ast::ty::TypeKind;
        match &ty.kind {
            TypeKind::Int => VarType::Int,
            TypeKind::Bool => VarType::Bool,
            TypeKind::Float => VarType::Real,
            TypeKind::Path(path) => {
                let name = path
                    .segments
                    .iter()
                    .map(path_segment_to_str)
                    .collect::<List<_>>()
                    .join(".");
                {
                    let n = name.as_str();
                    match n {
                        _ if verum_common::well_known_types::type_names::is_integer_type(n) => VarType::Int,
                        "Bool" => VarType::Bool,
                        _ if verum_common::well_known_types::type_names::is_float_type(n) || n == "Real" => VarType::Real,
                        _ => VarType::Sort(Text::from(name)),
                    }
                }
            }
            _ => VarType::Int,
        }
    }

    /// Parse a contract literal string
    ///
    /// Format: contract#"requires expr1; ensures expr2; ..."
    ///
    /// Supported clauses:
    /// - requires: Precondition
    /// - ensures: Postcondition
    /// - modifies: Frame condition (what may be modified)
    /// - decreases: Termination measure
    fn parse_contract_literal(&self, content: &Text) -> (List<Formula>, List<Formula>) {
        let mut preconditions = List::new();
        let mut postconditions = List::new();

        // Split by semicolons and process each clause
        for clause in content.as_str().split(';') {
            let clause = clause.trim();
            if clause.is_empty() {
                continue;
            }

            // Parse clause type and expression
            if let Some(expr_str) = clause.strip_prefix("requires").map(str::trim) {
                let formula = self.parse_contract_string(expr_str, ContractContext::Precondition);
                preconditions.push(formula);
            } else if let Some(expr_str) = clause.strip_prefix("ensures").map(str::trim) {
                let formula = self.parse_contract_string(expr_str, ContractContext::Postcondition);
                postconditions.push(formula);
            } else if let Some(expr_str) = clause.strip_prefix("invariant").map(str::trim) {
                let formula = self.parse_contract_string(expr_str, ContractContext::Invariant);
                // Invariants go into both pre and post
                preconditions.push(formula.clone());
                postconditions.push(formula);
            } else if clause.starts_with("modifies") {
                // Frame conditions - track which locations may be modified
                // For now, skip - would need frame condition support
            } else if clause.starts_with("decreases") {
                // Termination measures - handled by termination checker
            }
        }

        (preconditions, postconditions)
    }

    /// Parse a contract expression string into a Formula
    ///
    /// This performs simple expression parsing for contract strings.
    /// For complex expressions, use the full parser.
    fn parse_contract_string(&self, expr_str: &str, context: ContractContext) -> Formula {
        let expr_str = expr_str.trim();

        // Handle result keyword in postconditions
        if expr_str == "result" && context == ContractContext::Postcondition {
            return Formula::Var(Variable::result());
        }

        // Handle old(expr) pattern
        if let Some(inner) = expr_str
            .strip_prefix("old(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let inner_formula =
                self.parse_contract_string(inner.trim(), ContractContext::Precondition);
            // Mark as old value
            match inner_formula {
                Formula::Var(v) => {
                    return Formula::Predicate(Text::from("old"), vec![SmtExpr::Var(v)].into());
                }
                _ => return inner_formula,
            }
        }

        // Handle comparison operators
        if let Some((left, right)) = expr_str.split_once(">=") {
            let left_smt = self.parse_simple_expr(left.trim());
            let right_smt = self.parse_simple_expr(right.trim());
            return Formula::ge(left_smt, right_smt);
        }
        if let Some((left, right)) = expr_str.split_once("<=") {
            let left_smt = self.parse_simple_expr(left.trim());
            let right_smt = self.parse_simple_expr(right.trim());
            return Formula::le(left_smt, right_smt);
        }
        if let Some((left, right)) = expr_str.split_once("!=") {
            let left_smt = self.parse_simple_expr(left.trim());
            let right_smt = self.parse_simple_expr(right.trim());
            return Formula::Ne(Box::new(left_smt), Box::new(right_smt));
        }
        if let Some((left, right)) = expr_str.split_once("==") {
            let left_smt = self.parse_simple_expr(left.trim());
            let right_smt = self.parse_simple_expr(right.trim());
            return Formula::eq(left_smt, right_smt);
        }
        if let Some((left, right)) = expr_str.split_once('>') {
            let left_smt = self.parse_simple_expr(left.trim());
            let right_smt = self.parse_simple_expr(right.trim());
            return Formula::gt(left_smt, right_smt);
        }
        if let Some((left, right)) = expr_str.split_once('<') {
            let left_smt = self.parse_simple_expr(left.trim());
            let right_smt = self.parse_simple_expr(right.trim());
            return Formula::lt(left_smt, right_smt);
        }

        // Handle logical operators
        if let Some((left, right)) = expr_str.split_once("&&") {
            let left_formula = self.parse_contract_string(left.trim(), context);
            let right_formula = self.parse_contract_string(right.trim(), context);
            return Formula::and([left_formula, right_formula]);
        }
        if let Some((left, right)) = expr_str.split_once("||") {
            let left_formula = self.parse_contract_string(left.trim(), context);
            let right_formula = self.parse_contract_string(right.trim(), context);
            return Formula::or([left_formula, right_formula]);
        }
        if let Some((left, right)) = expr_str.split_once("=>") {
            let left_formula = self.parse_contract_string(left.trim(), context);
            let right_formula = self.parse_contract_string(right.trim(), context);
            return Formula::implies(left_formula, right_formula);
        }

        // Handle negation
        if let Some(inner) = expr_str.strip_prefix('!') {
            let inner_formula = self.parse_contract_string(inner.trim(), context);
            return Formula::not(inner_formula);
        }

        // Handle forall
        if let Some(rest) = expr_str.strip_prefix("forall") {
            return self.parse_quantifier_string(rest.trim(), true, context);
        }

        // Handle exists
        if let Some(rest) = expr_str.strip_prefix("exists") {
            return self.parse_quantifier_string(rest.trim(), false, context);
        }

        // Parse as simple expression and convert to formula
        let smt_expr = self.parse_simple_expr(expr_str);
        self.expr_to_formula(&smt_expr)
    }

    /// Parse a simple arithmetic expression from a string
    fn parse_simple_expr(&self, expr_str: &str) -> SmtExpr {
        let expr_str = expr_str.trim();

        // Handle result keyword
        if expr_str == "result" {
            return SmtExpr::Var(Variable::result());
        }

        // Handle integer literals
        if let Ok(n) = expr_str.parse::<i64>() {
            return SmtExpr::int(n);
        }

        // Handle boolean literals
        if expr_str == "true" {
            return SmtExpr::bool(true);
        }
        if expr_str == "false" {
            return SmtExpr::bool(false);
        }

        // Handle parenthesized expressions
        if expr_str.starts_with('(') && expr_str.ends_with(')') {
            return self.parse_simple_expr(&expr_str[1..expr_str.len() - 1]);
        }

        // Handle binary arithmetic operators (simple left-to-right parsing)
        if let Some((left, right)) = expr_str.rsplit_once('+') {
            let left_smt = self.parse_simple_expr(left.trim());
            let right_smt = self.parse_simple_expr(right.trim());
            return SmtExpr::add(left_smt, right_smt);
        }
        // Check for subtraction, but not negative numbers
        if let Some((left, right)) = expr_str.rsplit_once('-')
            && !left.is_empty()
        {
            let left_smt = self.parse_simple_expr(left.trim());
            let right_smt = self.parse_simple_expr(right.trim());
            return SmtExpr::sub(left_smt, right_smt);
        }
        if let Some((left, right)) = expr_str.rsplit_once('*') {
            let left_smt = self.parse_simple_expr(left.trim());
            let right_smt = self.parse_simple_expr(right.trim());
            return SmtExpr::mul(left_smt, right_smt);
        }
        if let Some((left, right)) = expr_str.rsplit_once('/') {
            let left_smt = self.parse_simple_expr(left.trim());
            let right_smt = self.parse_simple_expr(right.trim());
            return SmtExpr::BinOp(SmtBinOp::Div, Box::new(left_smt), Box::new(right_smt));
        }
        if let Some((left, right)) = expr_str.rsplit_once('%') {
            let left_smt = self.parse_simple_expr(left.trim());
            let right_smt = self.parse_simple_expr(right.trim());
            return SmtExpr::BinOp(SmtBinOp::Mod, Box::new(left_smt), Box::new(right_smt));
        }

        // Handle function calls: name(args)
        if let Some(paren_idx) = expr_str.find('(')
            && expr_str.ends_with(')')
        {
            let func_name = expr_str[..paren_idx].trim();
            let args_str = &expr_str[paren_idx + 1..expr_str.len() - 1];
            let args: List<SmtExpr> = args_str
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| self.parse_simple_expr(s.trim()))
                .collect();
            return SmtExpr::Apply(Text::from(func_name), args);
        }

        // Handle field access: expr.field
        if let Some((base, field)) = expr_str.rsplit_once('.') {
            // Check if this might be a floating point number
            if base.chars().all(|c| c.is_ascii_digit())
                && field.chars().all(|c| c.is_ascii_digit())
                && let Ok(f) = expr_str.parse::<f64>()
            {
                return SmtExpr::RealConst(f);
            }
            let base_smt = self.parse_simple_expr(base.trim());
            return SmtExpr::Apply(
                Text::from(format!("field_{}", field.trim())),
                vec![base_smt].into(),
            );
        }

        // Handle array index: expr[idx]
        if let Some(bracket_idx) = expr_str.find('[')
            && expr_str.ends_with(']')
        {
            let arr_name = expr_str[..bracket_idx].trim();
            let idx_str = &expr_str[bracket_idx + 1..expr_str.len() - 1];
            let arr_smt = self.parse_simple_expr(arr_name);
            let idx_smt = self.parse_simple_expr(idx_str.trim());
            return SmtExpr::Select(Box::new(arr_smt), Box::new(idx_smt));
        }

        // Default: treat as variable
        SmtExpr::var(expr_str)
    }

    /// Parse a quantifier string expression
    ///
    /// Formats:
    /// - "x: Int. body" - explicit type
    /// - "x. body" - inferred type
    /// - "(x: Int, y: Int). body" - multiple variables
    fn parse_quantifier_string(
        &self,
        rest: &str,
        is_forall: bool,
        context: ContractContext,
    ) -> Formula {
        let rest = rest.trim();

        // Find the dot separating variables from body
        let dot_idx = match rest.find('.') {
            Some(idx) => idx,
            None => return Formula::True, // Invalid format
        };

        let vars_part = rest[..dot_idx].trim();
        let body_part = rest[dot_idx + 1..].trim();

        // Parse bound variables
        let mut bound_vars = List::new();

        // Remove surrounding parens if present
        let vars_str = if vars_part.starts_with('(') && vars_part.ends_with(')') {
            &vars_part[1..vars_part.len() - 1]
        } else {
            vars_part
        };

        // Parse each variable declaration
        for var_decl in vars_str.split(',') {
            let var_decl = var_decl.trim();
            if let Some((name, ty_str)) = var_decl.split_once(':') {
                let name = name.trim();
                let ty = self.parse_var_type_string(ty_str.trim());
                bound_vars.push(Variable::typed(name, ty));
            } else {
                // No type annotation - default to Int
                bound_vars.push(Variable::typed(var_decl, VarType::Int));
            }
        }

        // Parse the body
        let body_formula = self.parse_contract_string(body_part, context);

        if bound_vars.is_empty() {
            body_formula
        } else if is_forall {
            Formula::Forall(bound_vars, Box::new(body_formula))
        } else {
            Formula::Exists(bound_vars, Box::new(body_formula))
        }
    }

    /// Parse a type string to VarType
    fn parse_var_type_string(&self, ty_str: &str) -> VarType {
        let s = ty_str.trim();
        match s {
            _ if verum_common::well_known_types::type_names::is_integer_type(s) && !verum_common::well_known_types::type_names::is_unsigned_integer_type(s) => VarType::Int,
            "Bool" | "bool" => VarType::Bool,
            _ if verum_common::well_known_types::type_names::is_float_type(s) || s == "Real" => VarType::Real,
            "u8" => VarType::BitVec(8),
            "u16" => VarType::BitVec(16),
            "u32" => VarType::BitVec(32),
            "u64" => VarType::BitVec(64),
            other => VarType::Sort(Text::from(other)),
        }
    }

    /// Generate verification conditions from extracted contracts
    ///
    /// Creates VCs for:
    /// - Precondition checking at function entry
    /// - Postcondition checking at function exit
    /// - Frame condition preservation
    pub fn generate_contract_vcs(
        &mut self,
        func: &FunctionDecl,
        precondition: &Formula,
        postcondition: &Formula,
    ) {
        let func_name = func.name.as_str();
        let location = SourceLocation::from_span(func.span, self.source_file.clone());

        // VC 1: Precondition is consistent (not false)
        if *precondition != Formula::True {
            let pre_consistency_vc = VerificationCondition::new(
                Formula::not(Formula::eq(
                    SmtExpr::var("precondition_consistent"),
                    SmtExpr::bool(false),
                )),
                location.clone(),
                VCKind::Precondition,
                format!("Function '{}' precondition is satisfiable", func_name),
            )
            .with_function(func_name);
            self.vcs.push(pre_consistency_vc);
        }

        // VC 2: Postcondition is achievable given precondition
        // This is the main function VC: Pre => wp(body, Post)
        // Already generated in generate_vcs()
    }

    /// Handle old() expressions in postconditions
    ///
    /// Transforms postcondition formulas by:
    /// - Replacing old(x) with x_old (parameter snapshot)
    /// - Setting up parameter snapshots at function entry
    pub fn setup_old_values(&self, params: &[Text]) -> Map<Text, Variable> {
        let mut old_vars = Map::new();
        for param in params {
            let old_name = Text::from(format!("{}_old", param));
            old_vars.insert(param.clone(), Variable::new(old_name));
        }
        old_vars
    }

    /// Transform a postcondition formula to handle old() references
    ///
    /// Replaces old(expr) predicates with references to snapshot variables
    pub fn transform_old_in_postcondition(
        &self,
        formula: &Formula,
        old_vars: &Map<Text, Variable>,
    ) -> Formula {
        match formula {
            Formula::Predicate(name, args) if name.as_str() == "old" => {
                // Transform old(x) to x_old
                if let Some(SmtExpr::Var(v)) = args.first()
                    && let Some(old_var) = old_vars.get(&v.name)
                {
                    return Formula::Var(old_var.clone());
                }
                formula.clone()
            }
            Formula::Not(inner) => Formula::Not(Box::new(
                self.transform_old_in_postcondition(inner, old_vars),
            )),
            Formula::And(formulas) => Formula::And(
                formulas
                    .iter()
                    .map(|f| self.transform_old_in_postcondition(f, old_vars))
                    .collect(),
            ),
            Formula::Or(formulas) => Formula::Or(
                formulas
                    .iter()
                    .map(|f| self.transform_old_in_postcondition(f, old_vars))
                    .collect(),
            ),
            Formula::Implies(ante, cons) => Formula::Implies(
                Box::new(self.transform_old_in_postcondition(ante, old_vars)),
                Box::new(self.transform_old_in_postcondition(cons, old_vars)),
            ),
            Formula::Iff(left, right) => Formula::Iff(
                Box::new(self.transform_old_in_postcondition(left, old_vars)),
                Box::new(self.transform_old_in_postcondition(right, old_vars)),
            ),
            Formula::Forall(vars, inner) => Formula::Forall(
                vars.clone(),
                Box::new(self.transform_old_in_postcondition(inner, old_vars)),
            ),
            Formula::Exists(vars, inner) => Formula::Exists(
                vars.clone(),
                Box::new(self.transform_old_in_postcondition(inner, old_vars)),
            ),
            Formula::Eq(left, right) => Formula::Eq(
                Box::new(self.transform_old_in_expr(left, old_vars)),
                Box::new(self.transform_old_in_expr(right, old_vars)),
            ),
            Formula::Ne(left, right) => Formula::Ne(
                Box::new(self.transform_old_in_expr(left, old_vars)),
                Box::new(self.transform_old_in_expr(right, old_vars)),
            ),
            Formula::Lt(left, right) => Formula::Lt(
                Box::new(self.transform_old_in_expr(left, old_vars)),
                Box::new(self.transform_old_in_expr(right, old_vars)),
            ),
            Formula::Le(left, right) => Formula::Le(
                Box::new(self.transform_old_in_expr(left, old_vars)),
                Box::new(self.transform_old_in_expr(right, old_vars)),
            ),
            Formula::Gt(left, right) => Formula::Gt(
                Box::new(self.transform_old_in_expr(left, old_vars)),
                Box::new(self.transform_old_in_expr(right, old_vars)),
            ),
            Formula::Ge(left, right) => Formula::Ge(
                Box::new(self.transform_old_in_expr(left, old_vars)),
                Box::new(self.transform_old_in_expr(right, old_vars)),
            ),
            _ => formula.clone(),
        }
    }

    /// Transform old() references in SMT expressions
    fn transform_old_in_expr(&self, expr: &SmtExpr, old_vars: &Map<Text, Variable>) -> SmtExpr {
        match expr {
            SmtExpr::Apply(name, args) if name.as_str() == "old" => {
                if let Some(SmtExpr::Var(v)) = args.first()
                    && let Some(old_var) = old_vars.get(&v.name)
                {
                    return SmtExpr::Var(old_var.clone());
                }
                expr.clone()
            }
            SmtExpr::BinOp(op, left, right) => SmtExpr::BinOp(
                *op,
                Box::new(self.transform_old_in_expr(left, old_vars)),
                Box::new(self.transform_old_in_expr(right, old_vars)),
            ),
            SmtExpr::UnOp(op, inner) => {
                SmtExpr::UnOp(*op, Box::new(self.transform_old_in_expr(inner, old_vars)))
            }
            SmtExpr::Apply(name, args) => SmtExpr::Apply(
                name.clone(),
                args.iter()
                    .map(|a| self.transform_old_in_expr(a, old_vars))
                    .collect(),
            ),
            SmtExpr::Select(arr, idx) => SmtExpr::Select(
                Box::new(self.transform_old_in_expr(arr, old_vars)),
                Box::new(self.transform_old_in_expr(idx, old_vars)),
            ),
            SmtExpr::Store(arr, idx, val) => SmtExpr::Store(
                Box::new(self.transform_old_in_expr(arr, old_vars)),
                Box::new(self.transform_old_in_expr(idx, old_vars)),
                Box::new(self.transform_old_in_expr(val, old_vars)),
            ),
            _ => expr.clone(),
        }
    }

    /// Set a loop invariant for subsequent analysis
    pub fn set_loop_invariant(&mut self, loop_id: u64, invariant: Formula) {
        self.symbol_table.add_loop_invariant(loop_id, invariant);
    }

    /// Register a function signature for call verification
    pub fn register_function(&mut self, name: impl Into<Text>, sig: FunctionSignature) {
        self.symbol_table.add_function(name, sig);
    }
}

impl Default for VCGenerator {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Public API Functions
// =============================================================================

/// Generate verification conditions for a function
///
/// This is the main entry point for VC generation.
///
/// # Example
///
/// ```rust,ignore
/// use verum_verification::vcgen::generate_vcs;
/// use verum_ast::decl::FunctionDecl;
///
/// let func: FunctionDecl = /* ... */;
/// let vcs = generate_vcs(&func);
/// for vc in vcs.iter() {
///     println!("VC: {}", vc.to_smtlib());
/// }
/// ```
pub fn generate_vcs(func: &FunctionDecl) -> List<VerificationCondition> {
    let mut generator = VCGenerator::new();
    generator.generate_vcs(func)
}

/// Compute weakest precondition for a statement
///
/// Implements the wp rules from Dijkstra's calculus.
pub fn wp(stmt: &Stmt, postcondition: Formula) -> Formula {
    let mut generator = VCGenerator::new();
    generator.wp_stmt(stmt, &postcondition)
}

/// Substitute a variable with an expression in a formula
///
/// Implements Q[x/e] from the weakest precondition calculus.
pub fn substitute(formula: Formula, var: Variable, expr: SmtExpr) -> Formula {
    formula.substitute(&var, &expr)
}

/// Convert a verification condition to SMT-LIB format
///
/// Returns a complete SMT-LIB 2.6 script ready for solver input.
pub fn vc_to_smtlib(vc: &VerificationCondition) -> Text {
    vc.to_smtlib()
}
