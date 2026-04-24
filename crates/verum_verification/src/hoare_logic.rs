//! # Hoare Logic Implementation for Verum
//!
//! This module implements Hoare Logic for formal verification of Verum programs.
//! It provides {P} c {Q} verification through weakest precondition (WP) calculus,
//! verification condition generation, and SMT integration.
//!
//! # Specification
//!
//! Hoare triples {P} c {Q} mean: if P holds before executing c, then Q holds after.
//! The weakest precondition wp(c, Q) gives the most liberal precondition guaranteeing Q.
//! WP rules: wp(skip, Q) = Q; wp(x:=e, Q) = Q[e/x]; wp(S1;S2, Q) = wp(S1, wp(S2, Q));
//! wp(if b then S1 else S2, Q) = (b => wp(S1,Q)) /\ (!b => wp(S2,Q));
//! wp(while b inv I, Q) = I /\ (I /\ b => wp(S, I)) /\ (I /\ !b => Q).
//!
//! # Theory
//!
//! Hoare triples `{P} c {Q}` consist of:
//! - P: Precondition (assertion before command execution)
//! - c: Command/Statement
//! - Q: Postcondition (assertion after command execution)
//!
//! The triple is valid if: whenever P holds before executing c, Q holds after.
//!
//! ## Weakest Precondition (WP) Calculus
//!
//! The WP calculus computes the weakest precondition that guarantees Q holds after c:
//!
//! ```text
//! wp(skip, Q) = Q
//! wp(x := e, Q) = Q[x := eval(e)]
//! wp(c1; c2, Q) = wp(c1, wp(c2, Q))
//! wp(if b then c1 else c2, Q) = (b => wp(c1, Q)) ∧ (¬b => wp(c2, Q))
//! wp(while b inv I, Q) = I ∧ ∀v. (I ∧ b => wp(c, I)) ∧ (I ∧ ¬b => Q)
//! ```
//!
//! # Examples
//!
//! ```no_run
//! use verum_verification::hoare_logic::{HoareTriple, HoareLogic, Command};
//! use verum_verification::vcgen::{Formula, SmtExpr, Variable};
//!
//! // Create a Hoare triple: {x >= 0} x := x + 1 {x > 0}
//! let pre = Formula::ge(SmtExpr::var("x"), SmtExpr::int(0));
//! let post = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));
//! let cmd = Command::Assign {
//!     var: Variable::new("x"),
//!     expr: SmtExpr::add(SmtExpr::var("x"), SmtExpr::int(1)),
//! };
//!
//! let triple = HoareTriple::new(pre, cmd, post);
//!
//! // Verify the triple using WP calculus
//! let logic = HoareLogic::new();
//! let vc = logic.generate_vc(&triple).unwrap();
//!
//! // Check VC with SMT solver
//! // let valid = logic.verify(&vc).unwrap();
//! ```

use crate::vcgen::{
    ContractContext, Formula, SmtBinOp, SmtExpr, SmtUnOp, SourceLocation, VarType, Variable,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use verum_ast::decl::FunctionDecl;
use verum_ast::expr::{BinOp, Block, Expr, ExprKind, UnOp};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::stmt::{Stmt, StmtKind};
use verum_ast::ty::PathSegment;
use verum_smt::counterexample::{CounterExample, CounterExampleExtractor};
// Use verum_common types to match verum_ast (List = Vec, Heap = Box, Maybe = Option)
use verum_common::{Heap, List, Map, Maybe, Text};
use z3::ast::{Array, Ast, BV, Bool, Dynamic, Int, Real};
use z3::{SatResult, Solver, Sort};

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
// Core Hoare Logic Types
// =============================================================================

/// A Hoare triple: {P} c {Q}
///
/// Represents a correctness specification for a command:
/// - P: Precondition (must hold before execution)
/// - c: Command (the program fragment)
/// - Q: Postcondition (must hold after execution)
///
/// The triple is valid iff: ∀σ. P(σ) => Q(⟦c⟧(σ))
/// where σ is a program state and ⟦c⟧ is the semantic function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HoareTriple {
    /// Precondition
    pub precondition: Formula,
    /// Command/statement
    pub command: Command,
    /// Postcondition
    pub postcondition: Formula,
    /// Source location for error reporting
    pub location: Maybe<SourceLocation>,
}

impl HoareTriple {
    /// Create a new Hoare triple
    pub fn new(precondition: Formula, command: Command, postcondition: Formula) -> Self {
        Self {
            precondition,
            command,
            postcondition,
            location: Maybe::None,
        }
    }

    /// Create a Hoare triple with source location
    pub fn with_location(
        precondition: Formula,
        command: Command,
        postcondition: Formula,
        location: SourceLocation,
    ) -> Self {
        Self {
            precondition,
            command,
            postcondition,
            location: Maybe::Some(location),
        }
    }

    /// Get a formatted representation of the triple
    pub fn format(&self) -> Text {
        Text::from(format!(
            "{{ {} }} {} {{ {} }}",
            self.precondition.to_smtlib(),
            self.command.format(),
            self.postcondition.to_smtlib()
        ))
    }
}

impl fmt::Display for HoareTriple {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format())
    }
}

/// Commands in Hoare logic
///
/// These represent the imperative core of the language.
/// Each command has a well-defined semantics in terms of state transformations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Skip (no-op): skip
    ///
    /// Semantics: ⟦skip⟧(σ) = σ
    Skip,

    /// Assignment: x := e
    ///
    /// Semantics: ⟦x := e⟧(σ) = σ[x ↦ ⟦e⟧(σ)]
    Assign {
        /// Variable being assigned
        var: Variable,
        /// Expression to assign
        expr: SmtExpr,
    },

    /// Sequential composition: c1; c2
    ///
    /// Semantics: ⟦c1; c2⟧(σ) = ⟦c2⟧(⟦c1⟧(σ))
    Seq {
        /// First command
        first: Heap<Command>,
        /// Second command
        second: Heap<Command>,
    },

    /// Conditional: if b then c1 else c2
    ///
    /// Semantics:
    /// ⟦if b then c1 else c2⟧(σ) = if ⟦b⟧(σ) then ⟦c1⟧(σ) else ⟦c2⟧(σ)
    If {
        /// Condition
        condition: Formula,
        /// Then branch
        then_branch: Heap<Command>,
        /// Else branch (optional)
        else_branch: Maybe<Heap<Command>>,
    },

    /// While loop: while b inv I do c
    ///
    /// Semantics: Fixed point of:
    /// ⟦while b do c⟧(σ) = if ⟦b⟧(σ) then ⟦while b do c⟧(⟦c⟧(σ)) else σ
    ///
    /// Requires loop invariant I for verification.
    While {
        /// Loop condition
        condition: Formula,
        /// Loop invariant (required for verification)
        invariant: Formula,
        /// Loop body
        body: Heap<Command>,
        /// Optional termination measure (decreases clause)
        /// Single measure for simple termination proofs
        decreases: Maybe<SmtExpr>,
        /// Optional lexicographic measures for complex termination proofs
        /// When provided, measures are compared lexicographically (first component has priority)
        /// Each measure must be well-founded (typically non-negative integers)
        lexicographic_decreases: Maybe<List<SmtExpr>>,
    },

    /// For loop: for x in range do c
    ///
    /// Desugars to while loop with appropriate bounds
    For {
        /// Loop variable
        var: Variable,
        /// Range start
        start: SmtExpr,
        /// Range end (exclusive)
        end: SmtExpr,
        /// Loop invariant
        invariant: Formula,
        /// Loop body
        body: Heap<Command>,
    },

    /// Assert statement: assert(P)
    ///
    /// Semantics: ⟦assert(P)⟧(σ) = if P(σ) then σ else error
    Assert(Formula),

    /// Assume statement: assume(P)
    ///
    /// Semantics: ⟦assume(P)⟧(σ) = if P(σ) then σ else undefined
    Assume(Formula),

    /// Havoc statement: havoc(x)
    ///
    /// Non-deterministically assigns to x.
    /// Semantics: ⟦havoc(x)⟧(σ) = σ[x ↦ *] where * is any value
    Havoc(Variable),

    /// Function call: x := f(args)
    ///
    /// Requires function contract for verification.
    Call {
        /// Variable to assign result (if any)
        result: Maybe<Variable>,
        /// Function name
        function: Text,
        /// Arguments
        args: List<SmtExpr>,
        /// Function contract (precondition)
        requires: Formula,
        /// Function contract (postcondition)
        ensures: Formula,
    },

    /// Array update: arr[idx] := val
    ArrayUpdate {
        /// Array variable
        array: Variable,
        /// Index expression
        index: SmtExpr,
        /// Value to store
        value: SmtExpr,
    },

    /// Block of commands
    Block(List<Command>),
}

impl Command {
    /// Get a formatted representation of the command
    pub fn format(&self) -> Text {
        match self {
            Command::Skip => Text::from("skip"),
            Command::Assign { var, expr } => Text::from(format!("{} := {}", var, expr.to_smtlib())),
            Command::Seq { first, second } => {
                Text::from(format!("{}; {}", first.format(), second.format()))
            }
            Command::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let else_str = match else_branch {
                    Maybe::Some(eb) => format!(" else {}", eb.format()),
                    Maybe::None => String::new(),
                };
                Text::from(format!(
                    "if {} then {}{}",
                    condition.to_smtlib(),
                    then_branch.format(),
                    else_str
                ))
            }
            Command::While {
                condition,
                invariant,
                body,
                ..
            } => Text::from(format!(
                "while {} inv {} do {}",
                condition.to_smtlib(),
                invariant.to_smtlib(),
                body.format()
            )),
            Command::For {
                var,
                start,
                end,
                body,
                ..
            } => Text::from(format!(
                "for {} in {}..{} do {}",
                var,
                start.to_smtlib(),
                end.to_smtlib(),
                body.format()
            )),
            Command::Assert(p) => Text::from(format!("assert({})", p.to_smtlib())),
            Command::Assume(p) => Text::from(format!("assume({})", p.to_smtlib())),
            Command::Havoc(v) => Text::from(format!("havoc({})", v)),
            Command::Call {
                result, function, ..
            } => {
                let res_str = match result {
                    Maybe::Some(v) => format!("{} := ", v),
                    Maybe::None => String::new(),
                };
                Text::from(format!("{}{}(...)", res_str, function))
            }
            Command::ArrayUpdate {
                array,
                index,
                value,
            } => Text::from(format!(
                "{}[{}] := {}",
                array,
                index.to_smtlib(),
                value.to_smtlib()
            )),
            Command::Block(cmds) => {
                let cmd_strs: List<Text> = cmds.iter().map(|c| c.format()).collect();
                Text::from(format!("{{ {} }}", cmd_strs.join("; ")))
            }
        }
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format())
    }
}

// =============================================================================
// Weakest Precondition (WP) Calculator
// =============================================================================

/// Weakest precondition calculator
///
/// Implements Dijkstra's weakest precondition calculus for Hoare logic.
/// Given a command c and postcondition Q, computes the weakest precondition
/// wp(c, Q) such that {wp(c, Q)} c {Q} is valid.
pub struct WPCalculator {
    /// Fresh variable counter for SSA
    var_counter: AtomicU64,
    /// Symbol table for variable types
    symbol_table: HashMap<Text, VarType>,
}

impl std::fmt::Debug for WPCalculator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WPCalculator")
            .field("var_counter", &self.var_counter.load(Ordering::SeqCst))
            .field("symbol_table", &self.symbol_table)
            .finish()
    }
}

impl WPCalculator {
    /// Create a new WP calculator
    pub fn new() -> Self {
        Self {
            var_counter: AtomicU64::new(0),
            symbol_table: HashMap::new(),
        }
    }

    /// Create a new WP calculator with symbol table
    pub fn with_symbols(symbol_table: HashMap<Text, VarType>) -> Self {
        Self {
            var_counter: AtomicU64::new(0),
            symbol_table,
        }
    }

    /// Generate a fresh variable for SSA
    fn fresh_var(&self, base: &str) -> Variable {
        let version = self.var_counter.fetch_add(1, Ordering::SeqCst);
        Variable::versioned(base, version)
    }

    /// Compute weakest precondition: wp(c, Q)
    ///
    /// Returns the weakest precondition that ensures Q holds after executing c.
    pub fn wp(&self, command: &Command, postcondition: &Formula) -> Result<Formula, WPError> {
        match command {
            // wp(skip, Q) = Q
            Command::Skip => Ok(postcondition.clone()),

            // wp(x := e, Q) = Q[x := e]
            Command::Assign { var, expr } => Ok(postcondition.substitute(var, expr)),

            // wp(c1; c2, Q) = wp(c1, wp(c2, Q))
            Command::Seq { first, second } => {
                let wp2 = self.wp(second, postcondition)?;
                self.wp(first, &wp2)
            }

            // wp(if b then c1 else c2, Q) = (b => wp(c1, Q)) ∧ (¬b => wp(c2, Q))
            Command::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let wp_then = self.wp(then_branch, postcondition)?;
                let wp_else = match else_branch {
                    Maybe::Some(eb) => self.wp(eb, postcondition)?,
                    Maybe::None => postcondition.clone(), // No else = skip
                };

                Ok(Formula::and(vec![
                    Formula::implies(condition.clone(), wp_then),
                    Formula::implies(Formula::not(condition.clone()), wp_else),
                ]))
            }

            // wp(while b inv I, Q) = I ∧ ∀v. (I ∧ b => wp(c, I)) ∧ (I ∧ ¬b => Q)
            //
            // For total correctness with termination, we additionally verify:
            // - Single measure: measure >= 0 ∧ measure decreases strictly
            // - Lexicographic: (m1, m2, ..., mn) decreases lexicographically
            //
            // Loop WP rule: wp(while b inv I, Q) = I /\ (I /\ b => wp(body, I)) /\ (I /\ !b => Q)
            // For total correctness with termination, additionally verify:
            // - Single measure: measure >= 0 and strictly decreases each iteration
            // - Lexicographic: (m1, m2, ..., mn) decreases lexicographically
            Command::While {
                condition,
                invariant,
                body,
                decreases,
                lexicographic_decreases,
            } => {
                let wp_body = self.wp(body, invariant)?;

                // Collect all variables modified in the loop body
                let modified_vars = self.collect_modified_vars(body);

                // Create fresh variables for universal quantification
                let fresh_vars: List<Variable> = modified_vars
                    .iter()
                    .map(|v| self.fresh_var(v.name.as_str()))
                    .collect();

                // Build the verification conditions:
                // 1. Invariant holds initially: I
                // 2. Invariant is preserved: ∀v. (I ∧ b => wp(c, I))
                // 3. Invariant + ¬b implies postcondition: I ∧ ¬b => Q
                let mut conditions = Vec::new();

                // Condition 1: I (invariant holds initially)
                conditions.push(invariant.clone());

                // Condition 2: ∀v. (I ∧ b => wp(c, I)) (invariant preservation)
                let preservation = Formula::implies(
                    Formula::and(vec![invariant.clone(), condition.clone()]),
                    wp_body,
                );
                if !fresh_vars.is_empty() {
                    let list_vars: List<Variable> = fresh_vars.into_iter().collect();
                    conditions.push(Formula::Forall(list_vars, Box::new(preservation)));
                } else {
                    conditions.push(preservation);
                }

                // Condition 3: I ∧ ¬b => Q (loop exit implies postcondition)
                conditions.push(Formula::implies(
                    Formula::and(vec![invariant.clone(), Formula::not(condition.clone())]),
                    postcondition.clone(),
                ));

                // =================================================================
                // Termination Verification Conditions
                // =================================================================
                //
                // For total correctness, we must prove the loop terminates.
                // This requires:
                // 1. A well-founded measure (non-negative for integers)
                // 2. Strict decrease of the measure on each iteration
                //
                // For lexicographic measures (m1, m2, ..., mn), the ordering is:
                // (m1', m2', ..., mn') < (m1, m2, ..., mn) iff
                //   m1' < m1, OR
                //   (m1' = m1 AND m2' < m2), OR
                //   (m1' = m1 AND m2' = m2 AND m3' < m3), OR
                //   ...
                // =================================================================

                // Check for lexicographic measures first (higher priority)
                if let Maybe::Some(measures) = lexicographic_decreases {
                    if !measures.is_empty() {
                        // Generate termination VCs for lexicographic ordering
                        let termination_vcs = self.generate_lexicographic_termination_vcs(
                            invariant, condition, body, measures,
                        )?;
                        conditions.extend(termination_vcs);
                    }
                } else if let Maybe::Some(measure) = decreases {
                    // Single measure termination verification
                    let termination_vcs = self.generate_single_measure_termination_vcs(
                        invariant, condition, body, measure,
                    )?;
                    conditions.extend(termination_vcs);
                }

                Ok(Formula::and(conditions))
            }

            // wp(for x in start..end, Q) - desugar to while loop
            Command::For {
                var,
                start,
                end,
                invariant,
                body,
            } => {
                // for x in start..end inv I { body }
                //   ≡
                // x := start; while x < end inv I { body; x := x + 1 }

                let init_assign = Command::Assign {
                    var: var.clone(),
                    expr: start.clone(),
                };

                let loop_cond = Formula::lt(SmtExpr::Var(var.clone()), end.clone());

                let incr = Command::Assign {
                    var: var.clone(),
                    expr: SmtExpr::add(SmtExpr::Var(var.clone()), SmtExpr::int(1)),
                };

                let loop_body = Command::Seq {
                    first: body.clone(),
                    second: Heap::new(incr),
                };

                let while_cmd = Command::While {
                    condition: loop_cond,
                    invariant: invariant.clone(),
                    body: Heap::new(loop_body),
                    decreases: Maybe::Some(SmtExpr::sub(end.clone(), SmtExpr::Var(var.clone()))),
                    lexicographic_decreases: Maybe::None,
                };

                let seq = Command::Seq {
                    first: Heap::new(init_assign),
                    second: Heap::new(while_cmd),
                };

                self.wp(&seq, postcondition)
            }

            // wp(assert(P), Q) = P ∧ Q
            Command::Assert(p) => Ok(Formula::and(vec![p.clone(), postcondition.clone()])),

            // wp(assume(P), Q) = P => Q
            Command::Assume(p) => Ok(Formula::implies(p.clone(), postcondition.clone())),

            // wp(havoc(x), Q) = ∀x. Q
            Command::Havoc(var) => Ok(Formula::Forall(
                vec![var.clone()].into(),
                Box::new(postcondition.clone()),
            )),

            // wp(x := f(args), Q) = requires(f) ∧ ∀result. ensures(f) => Q[x := result]
            Command::Call {
                result,
                requires,
                ensures,
                ..
            } => {
                let mut conditions = vec![requires.clone()];

                match result {
                    Maybe::Some(res_var) => {
                        let result_var = Variable::result();
                        let subst_q =
                            postcondition.substitute(res_var, &SmtExpr::Var(result_var.clone()));
                        let post_cond = Formula::Forall(
                            vec![result_var].into(),
                            Box::new(Formula::implies(ensures.clone(), subst_q)),
                        );
                        conditions.push(post_cond);
                    }
                    Maybe::None => {
                        conditions.push(ensures.clone());
                    }
                }

                Ok(Formula::and(conditions))
            }

            // wp(arr[idx] := val, Q) = Q[arr := store(arr, idx, val)]
            Command::ArrayUpdate {
                array,
                index,
                value,
            } => {
                let updated_array = SmtExpr::Store(
                    Box::new(SmtExpr::Var(array.clone())),
                    Box::new(index.clone()),
                    Box::new(value.clone()),
                );
                Ok(postcondition.substitute(array, &updated_array))
            }

            // wp(block, Q) = wp of the block as a sequence
            Command::Block(cmds) => {
                if cmds.is_empty() {
                    Ok(postcondition.clone())
                } else {
                    // Build sequential composition from the list
                    let mut cmd_iter = cmds.iter().rev();
                    // Safe: `cmds.is_empty()` was handled by the
                    // branch above, so `cmds.iter().rev().next()`
                    // yields Some on the first call. `.expect`
                    // documents the invariant.
                    let last = cmd_iter
                        .next()
                        .expect("cmds.is_empty() guard ensures at least one element");
                    let mut result = self.wp(last, postcondition)?;

                    for cmd in cmd_iter {
                        result = self.wp(cmd, &result)?;
                    }

                    Ok(result)
                }
            }
        }
    }

    /// Collect variables modified by a command
    fn collect_modified_vars(&self, command: &Command) -> HashSet<Variable> {
        let mut vars = HashSet::new();
        self.collect_modified_vars_rec(command, &mut vars);
        vars
    }

    fn collect_modified_vars_rec(&self, command: &Command, vars: &mut HashSet<Variable>) {
        match command {
            Command::Assign { var, .. } => {
                vars.insert(var.clone());
            }
            Command::Seq { first, second } => {
                self.collect_modified_vars_rec(first, vars);
                self.collect_modified_vars_rec(second, vars);
            }
            Command::If {
                then_branch,
                else_branch,
                ..
            } => {
                self.collect_modified_vars_rec(then_branch, vars);
                if let Maybe::Some(eb) = else_branch {
                    self.collect_modified_vars_rec(eb, vars);
                }
            }
            Command::While { body, .. } | Command::For { body, .. } => {
                self.collect_modified_vars_rec(body, vars);
            }
            Command::Call { result, .. } => {
                if let Maybe::Some(v) = result {
                    vars.insert(v.clone());
                }
            }
            Command::ArrayUpdate { array, .. } => {
                vars.insert(array.clone());
            }
            Command::Havoc(v) => {
                vars.insert(v.clone());
            }
            Command::Block(cmds) => {
                for cmd in cmds.iter() {
                    self.collect_modified_vars_rec(cmd, vars);
                }
            }
            Command::Skip | Command::Assert(_) | Command::Assume(_) => {}
        }
    }

    // =========================================================================
    // Termination Verification
    // =========================================================================

    /// Generate termination verification conditions for a single measure
    ///
    /// For a loop with measure M, generates:
    /// 1. Well-foundedness: I ∧ b => M >= 0
    /// 2. Strict decrease: I ∧ b => wp(body, M' < M)
    ///
    /// where M' is the measure after executing the body.
    ///
    /// Generates termination VCs for a single measure expression:
    /// 1. Well-foundedness: I /\ b => M >= 0
    /// 2. Strict decrease: I /\ b => wp(body, M' < M)
    fn generate_single_measure_termination_vcs(
        &self,
        invariant: &Formula,
        condition: &Formula,
        body: &Command,
        measure: &SmtExpr,
    ) -> Result<Vec<Formula>, WPError> {
        let mut vcs = Vec::new();

        // Create a fresh variable to represent the measure value before body execution
        let measure_before_name = format!(
            "__measure_before_{}",
            self.var_counter.fetch_add(1, Ordering::SeqCst)
        );
        let measure_before = SmtExpr::var(measure_before_name.as_str());
        let measure_before_var = Variable::typed(measure_before_name.as_str(), VarType::Int);

        // VC1: Well-foundedness - measure is non-negative when loop continues
        // I ∧ b => measure >= 0
        let well_foundedness = Formula::implies(
            Formula::and(vec![invariant.clone(), condition.clone()]),
            Formula::ge(measure.clone(), SmtExpr::int(0)),
        );
        vcs.push(well_foundedness);

        // VC2: Strict decrease - measure strictly decreases after body execution
        // ∀measure_before. (I ∧ b ∧ measure = measure_before) => wp(body, measure < measure_before)
        //
        // This ensures that after executing the body, the new measure value is strictly
        // less than the old measure value.
        let measure_decreases_post = Formula::lt(measure.clone(), measure_before.clone());
        let termination_wp = self.wp(body, &measure_decreases_post)?;

        let measure_snapshot = Formula::eq(measure.clone(), measure_before.clone());
        let termination_cond = Formula::Forall(
            vec![measure_before_var].into(),
            Box::new(Formula::implies(
                Formula::and(vec![invariant.clone(), condition.clone(), measure_snapshot]),
                termination_wp,
            )),
        );
        vcs.push(termination_cond);

        Ok(vcs)
    }

    /// Generate termination verification conditions for lexicographic measures
    ///
    /// For measures (m1, m2, ..., mn), the lexicographic ordering is:
    /// (m1', m2', ..., mn') < (m1, m2, ..., mn) iff
    ///   m1' < m1, OR
    ///   (m1' = m1 AND m2' < m2), OR
    ///   (m1' = m1 AND m2' = m2 AND m3' < m3), OR
    ///   ...
    ///
    /// We generate:
    /// 1. Well-foundedness for all measures: I ∧ b => (m1 >= 0 ∧ m2 >= 0 ∧ ... ∧ mn >= 0)
    /// 2. Lexicographic decrease: I ∧ b => wp(body, lexicographic_lt(measures', measures))
    ///
    /// Generates termination VCs for lexicographic measures:
    /// 1. Well-foundedness: I /\ b => (m1 >= 0 /\ m2 >= 0 /\ ... /\ mn >= 0)
    /// 2. Lexicographic decrease: I /\ b => wp(body, lex_lt(measures', measures))
    fn generate_lexicographic_termination_vcs(
        &self,
        invariant: &Formula,
        condition: &Formula,
        body: &Command,
        measures: &List<SmtExpr>,
    ) -> Result<Vec<Formula>, WPError> {
        let mut vcs = Vec::new();

        if measures.is_empty() {
            return Ok(vcs);
        }

        // Create fresh variables for all measure values before body execution
        let base_id = self
            .var_counter
            .fetch_add(measures.len() as u64, Ordering::SeqCst);
        let measures_before: Vec<(SmtExpr, Variable)> = measures
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let name = format!("__measure_before_{}_{}", base_id, i);
                let var = Variable::typed(name.as_str(), VarType::Int);
                (SmtExpr::var(name.as_str()), var)
            })
            .collect();

        // VC1: Well-foundedness - all measures are non-negative
        // I ∧ b => (m1 >= 0 ∧ m2 >= 0 ∧ ... ∧ mn >= 0)
        let non_neg_conditions: Vec<Formula> = measures
            .iter()
            .map(|m| Formula::ge(m.clone(), SmtExpr::int(0)))
            .collect();
        let well_foundedness = Formula::implies(
            Formula::and(vec![invariant.clone(), condition.clone()]),
            Formula::and(non_neg_conditions),
        );
        vcs.push(well_foundedness);

        // VC2: Lexicographic decrease
        // Build the lexicographic less-than formula for post-body measures
        let lex_decrease = self.build_lexicographic_decrease(
            measures,
            &measures_before
                .iter()
                .map(|(e, _)| e.clone())
                .collect::<Vec<_>>(),
        );

        // Compute wp(body, lex_decrease)
        let termination_wp = self.wp(body, &lex_decrease)?;

        // Build the snapshot conditions: measure_i = measure_before_i for all i
        let snapshot_conditions: Vec<Formula> = measures
            .iter()
            .zip(measures_before.iter())
            .map(|(m, (m_before, _))| Formula::eq(m.clone(), m_before.clone()))
            .collect();

        // Extract just the variables for quantification
        let quantified_vars: List<Variable> =
            measures_before.iter().map(|(_, var)| var.clone()).collect();

        // Build the complete termination VC with quantification
        let mut premises = vec![invariant.clone(), condition.clone()];
        premises.extend(snapshot_conditions);

        let termination_cond = Formula::Forall(
            quantified_vars,
            Box::new(Formula::implies(Formula::and(premises), termination_wp)),
        );
        vcs.push(termination_cond);

        Ok(vcs)
    }

    /// Build a formula representing lexicographic less-than
    ///
    /// Given current measures (m1, m2, ..., mn) and before measures (b1, b2, ..., bn),
    /// constructs:
    ///   (m1 < b1) OR
    ///   (m1 = b1 AND m2 < b2) OR
    ///   (m1 = b1 AND m2 = b2 AND m3 < b3) OR
    ///   ...
    ///
    /// This is the standard lexicographic ordering used in termination proofs.
    fn build_lexicographic_decrease(
        &self,
        current_measures: &List<SmtExpr>,
        before_measures: &[SmtExpr],
    ) -> Formula {
        if current_measures.is_empty() {
            // Empty tuple - no decrease possible (always false)
            return Formula::False;
        }

        let mut disjuncts = Vec::new();
        let mut equality_prefix = Vec::new();

        for (i, (current, before)) in current_measures
            .iter()
            .zip(before_measures.iter())
            .enumerate()
        {
            // Build: prefix_equalities AND current_i < before_i
            let strict_decrease = Formula::lt(current.clone(), before.clone());

            if i == 0 {
                // First component: just m1' < m1
                disjuncts.push(strict_decrease);
            } else {
                // i-th component: (m1' = m1 AND ... AND m_{i-1}' = m_{i-1} AND mi' < mi)
                let mut conjuncts = equality_prefix.clone();
                conjuncts.push(strict_decrease);
                disjuncts.push(Formula::and(conjuncts));
            }

            // Add equality for this position to the prefix for future iterations
            equality_prefix.push(Formula::eq(current.clone(), before.clone()));
        }

        Formula::or(disjuncts)
    }

    /// Validate that a measure expression is well-formed
    ///
    /// A well-formed measure must:
    /// 1. Be of an ordered type (integers, natural numbers, tuples thereof)
    /// 2. Have a well-founded ordering (non-negative integers satisfy this)
    ///
    /// Returns true if the measure appears to be well-formed.
    #[allow(dead_code)]
    fn validate_measure(&self, measure: &SmtExpr) -> bool {
        // For now, we accept any integer expression as a valid measure
        // The well-foundedness is enforced by the >= 0 verification condition
        //
        // Future enhancements could:
        // - Check type information from the symbol table
        // - Support custom well-founded orderings
        // - Handle ordinal measures for more complex termination proofs
        match measure {
            SmtExpr::IntConst(n) => *n >= 0,
            SmtExpr::Var(_) => true, // Variables are assumed to be integers
            SmtExpr::BinOp(SmtBinOp::Sub, _, _) => true, // Subtraction is valid
            SmtExpr::BinOp(SmtBinOp::Add, _, _) => true, // Addition is valid
            SmtExpr::BinOp(SmtBinOp::Mul, _, _) => true, // Multiplication is valid
            SmtExpr::UnOp(SmtUnOp::Len, _) => true, // Length is always non-negative
            SmtExpr::UnOp(SmtUnOp::Abs, _) => true, // Absolute value is always non-negative
            _ => true,               // Accept other expressions and let SMT solver validate
        }
    }

    /// Add variable type information
    pub fn add_symbol(&mut self, name: Text, ty: VarType) {
        self.symbol_table.insert(name, ty);
    }
}

impl Default for WPCalculator {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Hoare Logic Verifier
// =============================================================================

/// Main Hoare logic verification engine
///
/// Provides verification of Hoare triples using WP calculus and SMT solving.
#[derive(Debug)]
pub struct HoareLogic {
    /// WP calculator
    wp_calculator: WPCalculator,
    /// Statistics
    stats: HoareStats,
}

impl HoareLogic {
    /// Create a new Hoare logic verifier
    pub fn new() -> Self {
        Self {
            wp_calculator: WPCalculator::new(),
            stats: HoareStats::default(),
        }
    }

    /// Create with symbol table
    pub fn with_symbols(symbol_table: HashMap<Text, VarType>) -> Self {
        Self {
            wp_calculator: WPCalculator::with_symbols(symbol_table),
            stats: HoareStats::default(),
        }
    }

    /// Generate verification condition from a Hoare triple
    ///
    /// Returns a formula whose validity implies the triple is correct.
    /// VC: P => wp(c, Q)
    pub fn generate_vc(&self, triple: &HoareTriple) -> Result<VerificationCondition, WPError> {
        let wp_q = self
            .wp_calculator
            .wp(&triple.command, &triple.postcondition)?;
        let vc_formula = Formula::implies(triple.precondition.clone(), wp_q);

        Ok(VerificationCondition {
            formula: vc_formula,
            triple: triple.clone(),
            kind: VCKind::HoareTriple,
            location: triple.location.clone(),
        })
    }

    /// Verify a Hoare triple using WP calculus and SMT
    ///
    /// Returns `Ok(true)` if the triple is valid (VC is proven),
    /// or an error if verification failed or could not be completed.
    ///
    /// # Algorithm
    ///
    /// 1. Generate verification condition: P => wp(c, Q)
    /// 2. Convert VC formula to Z3 AST
    /// 3. Assert the negation of the VC (to check validity via UNSAT)
    /// 4. Call solver.check():
    ///    - UNSAT: VC is valid (original formula is a tautology)
    ///    - SAT: VC is invalid (counterexample exists)
    ///    - Unknown: Timeout or resource limit
    ///
    /// # Performance
    ///
    /// Typical verification times:
    /// - Simple arithmetic: <10ms
    /// - Loop invariants: 10-100ms
    /// - Complex predicates: 100-500ms
    pub fn verify(&mut self, triple: &HoareTriple) -> Result<bool, WPError> {
        let start_time = Instant::now();

        // Generate the verification condition
        let vc = self.generate_vc(triple)?;
        self.stats.vc_count += 1;

        // Collect free variables from the formula for counterexample extraction
        let free_vars = vc.formula.free_variables();
        let var_names: List<Text> = free_vars.iter().map(|v| v.name.clone()).collect();

        // Create Z3 solver
        let solver = Solver::new();

        // Convert the VC formula to Z3 Bool AST
        let z3_formula = self.formula_to_z3(&vc.formula)?;

        // To check validity of a formula phi, we check if NOT(phi) is UNSAT
        // If NOT(phi) is UNSAT, then phi is a tautology (always true)
        // If NOT(phi) is SAT, we have a counterexample showing phi can be false
        solver.assert(z3_formula.not());

        // Check satisfiability
        let result = solver.check();
        let elapsed = start_time.elapsed();
        self.stats.total_time_ms += elapsed.as_millis() as u64;

        match result {
            SatResult::Unsat => {
                // NOT(phi) is UNSAT means phi is valid
                // The Hoare triple is verified!
                self.stats.verified_count += 1;
                Ok(true)
            }
            SatResult::Sat => {
                // NOT(phi) is SAT means phi is invalid
                // Extract counterexample from the model
                self.stats.failed_count += 1;

                // Try to extract counterexample for better error reporting
                let counterexample = match solver.get_model() {
                    Some(model) => {
                        let extractor = CounterExampleExtractor::new(&model);
                        let ce =
                            extractor.extract(&var_names, &format!("{}", vc.formula.to_smtlib()));
                        Maybe::Some(ce)
                    }
                    None => Maybe::None,
                };

                // Return error with counterexample
                Err(WPError::VerificationFailed {
                    message: Text::from(format!(
                        "Hoare triple verification failed: {}",
                        triple.format()
                    )),
                    counterexample,
                    location: triple.location.clone(),
                })
            }
            SatResult::Unknown => {
                // Solver couldn't determine the result (timeout, etc.)
                self.stats.unknown_count += 1;

                let reason = solver
                    .get_reason_unknown()
                    .unwrap_or_else(|| "unknown".to_string());

                Err(WPError::Unknown {
                    reason: Text::from(reason),
                    location: triple.location.clone(),
                })
            }
        }
    }

    /// Convert a Formula to Z3 Bool AST
    ///
    /// Recursively translates the verification formula into Z3's internal
    /// representation for SMT solving.
    fn formula_to_z3(&self, formula: &Formula) -> Result<Bool, WPError> {
        match formula {
            Formula::True => Ok(Bool::from_bool(true)),
            Formula::False => Ok(Bool::from_bool(false)),

            Formula::Var(v) => {
                // Create a boolean variable with the variable name
                Ok(Bool::new_const(v.smtlib_name().as_str()))
            }

            Formula::Not(inner) => {
                let inner_z3 = self.formula_to_z3(inner)?;
                Ok(inner_z3.not())
            }

            Formula::And(formulas) => {
                if formulas.is_empty() {
                    return Ok(Bool::from_bool(true));
                }
                let z3_formulas: Result<Vec<Bool>, WPError> =
                    formulas.iter().map(|f| self.formula_to_z3(f)).collect();
                let z3_formulas = z3_formulas?;
                let refs: Vec<&Bool> = z3_formulas.iter().collect();
                Ok(Bool::and(&refs))
            }

            Formula::Or(formulas) => {
                if formulas.is_empty() {
                    return Ok(Bool::from_bool(false));
                }
                let z3_formulas: Result<Vec<Bool>, WPError> =
                    formulas.iter().map(|f| self.formula_to_z3(f)).collect();
                let z3_formulas = z3_formulas?;
                let refs: Vec<&Bool> = z3_formulas.iter().collect();
                Ok(Bool::or(&refs))
            }

            Formula::Implies(ante, cons) => {
                let ante_z3 = self.formula_to_z3(ante)?;
                let cons_z3 = self.formula_to_z3(cons)?;
                Ok(ante_z3.implies(&cons_z3))
            }

            Formula::Iff(left, right) => {
                let left_z3 = self.formula_to_z3(left)?;
                let right_z3 = self.formula_to_z3(right)?;
                Ok(left_z3.iff(&right_z3))
            }

            Formula::Forall(vars, inner) => {
                // Create Z3 bound variables
                let bound_vars: Vec<Dynamic> = vars
                    .iter()
                    .map(|v| match &v.ty {
                        Some(VarType::Bool) => {
                            Dynamic::from_ast(&Bool::new_const(v.smtlib_name().as_str()))
                        }
                        Some(VarType::Real) => {
                            Dynamic::from_ast(&Real::new_const(v.smtlib_name().as_str()))
                        }
                        _ => {
                            // Default to Int
                            Dynamic::from_ast(&Int::new_const(v.smtlib_name().as_str()))
                        }
                    })
                    .collect();

                let inner_z3 = self.formula_to_z3(inner)?;
                // Convert to &dyn Ast for forall_const
                let bound_refs: Vec<&dyn Ast> = bound_vars.iter().map(|v| v as &dyn Ast).collect();

                // Create the forall quantifier
                Ok(z3::ast::forall_const(&bound_refs, &[], &inner_z3))
            }

            Formula::Exists(vars, inner) => {
                // Create Z3 bound variables
                let bound_vars: Vec<Dynamic> = vars
                    .iter()
                    .map(|v| match &v.ty {
                        Some(VarType::Bool) => {
                            Dynamic::from_ast(&Bool::new_const(v.smtlib_name().as_str()))
                        }
                        Some(VarType::Real) => {
                            Dynamic::from_ast(&Real::new_const(v.smtlib_name().as_str()))
                        }
                        _ => {
                            // Default to Int
                            Dynamic::from_ast(&Int::new_const(v.smtlib_name().as_str()))
                        }
                    })
                    .collect();

                let inner_z3 = self.formula_to_z3(inner)?;
                // Convert to &dyn Ast for exists_const
                let bound_refs: Vec<&dyn Ast> = bound_vars.iter().map(|v| v as &dyn Ast).collect();

                // Create the exists quantifier
                Ok(z3::ast::exists_const(&bound_refs, &[], &inner_z3))
            }

            Formula::Eq(left, right) => {
                let left_z3 = self.expr_to_z3(left)?;
                let right_z3 = self.expr_to_z3(right)?;
                Ok(left_z3.eq(&right_z3))
            }

            Formula::Ne(left, right) => {
                let left_z3 = self.expr_to_z3(left)?;
                let right_z3 = self.expr_to_z3(right)?;
                Ok(left_z3.eq(&right_z3).not())
            }

            Formula::Lt(left, right) => {
                let left_z3 = self.expr_to_z3_int(left)?;
                let right_z3 = self.expr_to_z3_int(right)?;
                Ok(left_z3.lt(&right_z3))
            }

            Formula::Le(left, right) => {
                let left_z3 = self.expr_to_z3_int(left)?;
                let right_z3 = self.expr_to_z3_int(right)?;
                Ok(left_z3.le(&right_z3))
            }

            Formula::Gt(left, right) => {
                let left_z3 = self.expr_to_z3_int(left)?;
                let right_z3 = self.expr_to_z3_int(right)?;
                Ok(left_z3.gt(&right_z3))
            }

            Formula::Ge(left, right) => {
                let left_z3 = self.expr_to_z3_int(left)?;
                let right_z3 = self.expr_to_z3_int(right)?;
                Ok(left_z3.ge(&right_z3))
            }

            Formula::Predicate(name, args) => {
                // Create an uninterpreted function application
                // For now, just create a fresh boolean constant
                let const_name = format!(
                    "{}({})",
                    name,
                    args.iter()
                        .map(|a| a.to_smtlib().to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                );
                Ok(Bool::new_const(const_name.as_str()))
            }

            Formula::Let(var, bound_expr, body) => {
                // For let bindings, we substitute in the body
                let substituted = body.substitute(var, bound_expr);
                self.formula_to_z3(&substituted)
            }
        }
    }

    /// Convert an SmtExpr to Z3 Dynamic AST (generic)
    fn expr_to_z3(&self, expr: &SmtExpr) -> Result<Dynamic, WPError> {
        match expr {
            SmtExpr::Var(v) => {
                // Default to Int for untyped variables
                Ok(Dynamic::from_ast(&Int::new_const(v.smtlib_name().as_str())))
            }
            SmtExpr::IntConst(n) => Ok(Dynamic::from_ast(&Int::from_i64(*n))),
            SmtExpr::BoolConst(b) => Ok(Dynamic::from_ast(&Bool::from_bool(*b))),
            SmtExpr::RealConst(r) => {
                // Convert to rational representation
                let scaled = (*r * 1_000_000.0) as i64;
                Ok(Dynamic::from_ast(&Real::from_rational(scaled, 1_000_000)))
            }
            SmtExpr::BitVecConst(value, width) => {
                // Create Z3 bitvector constant with specified width
                Ok(Dynamic::from_ast(&BV::from_u64(*value, *width)))
            }
            SmtExpr::BinOp(op, left, right) => {
                // Check if we're dealing with bitvector operands
                let is_bv = matches!(left.as_ref(), SmtExpr::BitVecConst(_, _))
                    || matches!(right.as_ref(), SmtExpr::BitVecConst(_, _));

                if is_bv {
                    // Handle bitvector operations
                    let left_z3 = self.expr_to_z3_bv(left)?;
                    let right_z3 = self.expr_to_z3_bv(right)?;
                    match op {
                        SmtBinOp::Add => Ok(Dynamic::from_ast(&left_z3.bvadd(&right_z3))),
                        SmtBinOp::Sub => Ok(Dynamic::from_ast(&left_z3.bvsub(&right_z3))),
                        SmtBinOp::Mul => Ok(Dynamic::from_ast(&left_z3.bvmul(&right_z3))),
                        SmtBinOp::Div => Ok(Dynamic::from_ast(&left_z3.bvsdiv(&right_z3))),
                        SmtBinOp::Mod => Ok(Dynamic::from_ast(&left_z3.bvsmod(&right_z3))),
                        SmtBinOp::Pow => {
                            // Power not directly supported for bitvectors, model as uninterpreted
                            let const_name = format!("bvpow({},{})", left_z3, right_z3);
                            Ok(Dynamic::from_ast(&BV::new_const(
                                const_name.as_str(),
                                left_z3.get_size(),
                            )))
                        }
                        SmtBinOp::Select => {
                            // Array/tuple selection for bitvector arrays
                            self.expr_to_z3_select(left, right)
                        }
                    }
                } else {
                    // Handle integer operations
                    let left_z3 = self.expr_to_z3_int(left)?;
                    let right_z3 = self.expr_to_z3_int(right)?;
                    match op {
                        SmtBinOp::Add => Ok(Dynamic::from_ast(&Int::add(&[&left_z3, &right_z3]))),
                        SmtBinOp::Sub => Ok(Dynamic::from_ast(&Int::sub(&[&left_z3, &right_z3]))),
                        SmtBinOp::Mul => Ok(Dynamic::from_ast(&Int::mul(&[&left_z3, &right_z3]))),
                        SmtBinOp::Div => Ok(Dynamic::from_ast(&left_z3.div(&right_z3))),
                        SmtBinOp::Mod => Ok(Dynamic::from_ast(&left_z3.modulo(&right_z3))),
                        SmtBinOp::Pow => {
                            // Power returns Real in Z3, convert to Real first
                            let left_real = left_z3.to_real();
                            let right_real = right_z3.to_real();
                            Ok(Dynamic::from_ast(&left_real.power(&right_real)))
                        }
                        SmtBinOp::Select => {
                            // Array/tuple selection for integer arrays
                            self.expr_to_z3_select(left, right)
                        }
                    }
                }
            }
            SmtExpr::UnOp(op, inner) => {
                match op {
                    SmtUnOp::Neg => {
                        let inner_z3 = self.expr_to_z3_int(inner)?;
                        Ok(Dynamic::from_ast(&inner_z3.unary_minus()))
                    }
                    SmtUnOp::Abs => {
                        let inner_z3 = self.expr_to_z3_int(inner)?;
                        // abs(x) = if x >= 0 then x else -x
                        let zero = Int::from_i64(0);
                        Ok(Dynamic::from_ast(
                            &inner_z3.ge(&zero).ite(&inner_z3, &inner_z3.unary_minus()),
                        ))
                    }
                    SmtUnOp::Deref | SmtUnOp::Len | SmtUnOp::GetVariantValue => {
                        // These operations are not directly supported in Z3 integer context
                        // Model as uninterpreted functions
                        let inner_z3 = self.expr_to_z3(inner)?;
                        let op_name = match op {
                            SmtUnOp::Deref => "deref",
                            SmtUnOp::Len => "len",
                            SmtUnOp::GetVariantValue => "get_variant_value",
                            _ => unreachable!(),
                        };
                        let const_name = format!("{}({})", op_name, inner_z3);
                        Ok(Dynamic::from_ast(&Int::new_const(const_name.as_str())))
                    }
                }
            }
            SmtExpr::Apply(name, args) => {
                // Create an uninterpreted function application
                let const_name = format!(
                    "{}({})",
                    name,
                    args.iter()
                        .map(|a| a.to_smtlib().to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                );
                Ok(Dynamic::from_ast(&Int::new_const(const_name.as_str())))
            }
            SmtExpr::Select(arr, idx) => {
                // Array select: arr[idx] using proper Z3 array theory
                self.expr_to_z3_select(arr, idx)
            }
            SmtExpr::Store(arr, idx, val) => {
                // Array store: arr[idx := val] using proper Z3 array theory
                self.expr_to_z3_store(arr, idx, val)
            }
            SmtExpr::Ite(cond, then_e, else_e) => {
                let cond_z3 = self.formula_to_z3(cond)?;
                let then_z3 = self.expr_to_z3_int(then_e)?;
                let else_z3 = self.expr_to_z3_int(else_e)?;
                Ok(Dynamic::from_ast(&cond_z3.ite(&then_z3, &else_z3)))
            }
            SmtExpr::Let(var, bound_expr, body) => {
                // Substitute and convert
                let substituted = body.substitute(var, bound_expr);
                self.expr_to_z3(&substituted)
            }
        }
    }

    /// Convert an SmtExpr to Z3 Int AST (integer-specific)
    fn expr_to_z3_int(&self, expr: &SmtExpr) -> Result<Int, WPError> {
        match expr {
            SmtExpr::Var(v) => Ok(Int::new_const(v.smtlib_name().as_str())),
            SmtExpr::IntConst(n) => Ok(Int::from_i64(*n)),
            SmtExpr::BoolConst(_) => Err(WPError::TypeError(Text::from(
                "Expected integer expression, got boolean",
            ))),
            SmtExpr::RealConst(r) => {
                // Truncate to integer
                Ok(Int::from_i64(*r as i64))
            }
            SmtExpr::BitVecConst(value, width) => {
                // Convert bitvector to integer (signed interpretation)
                let bv = BV::from_u64(*value, *width);
                Ok(bv.to_int(true))
            }
            SmtExpr::BinOp(op, left, right) => {
                let left_z3 = self.expr_to_z3_int(left)?;
                let right_z3 = self.expr_to_z3_int(right)?;
                match op {
                    SmtBinOp::Add => Ok(Int::add(&[&left_z3, &right_z3])),
                    SmtBinOp::Sub => Ok(Int::sub(&[&left_z3, &right_z3])),
                    SmtBinOp::Mul => Ok(Int::mul(&[&left_z3, &right_z3])),
                    SmtBinOp::Div => Ok(left_z3.div(&right_z3)),
                    SmtBinOp::Mod => Ok(left_z3.modulo(&right_z3)),
                    SmtBinOp::Pow => {
                        // Power returns Real, but we need Int. For integer exponents,
                        // use multiplication or return error for large exponents.
                        // For now, we convert through Real and truncate.
                        let left_real = left_z3.to_real();
                        let right_real = right_z3.to_real();
                        let pow_result = left_real.power(&right_real);
                        Ok(pow_result.to_int())
                    }
                    SmtBinOp::Select => {
                        // Select operation on arrays - try to use proper array select
                        // or fall back to uninterpreted function
                        let arr_name = match left.as_ref() {
                            SmtExpr::Var(v) => v.smtlib_name(),
                            _ => Text::from("arr"),
                        };
                        let idx_str = right.to_smtlib();
                        Ok(Int::new_const(
                            format!("{}[{}]", arr_name, idx_str).as_str(),
                        ))
                    }
                }
            }
            SmtExpr::UnOp(op, inner) => {
                let inner_z3 = self.expr_to_z3_int(inner)?;
                Ok(match op {
                    SmtUnOp::Neg => inner_z3.unary_minus(),
                    SmtUnOp::Abs => {
                        let zero = Int::from_i64(0);
                        inner_z3.ge(&zero).ite(&inner_z3, &inner_z3.unary_minus())
                    }
                    SmtUnOp::Deref => {
                        // Dereference - return uninterpreted integer for the dereferenced value
                        Int::new_const(format!("deref_{}", inner_z3).as_str())
                    }
                    SmtUnOp::Len => {
                        // Length - return uninterpreted integer representing length
                        Int::new_const(format!("len_{}", inner_z3).as_str())
                    }
                    SmtUnOp::GetVariantValue => {
                        // Get variant value - return uninterpreted integer
                        Int::new_const(format!("variant_{}", inner_z3).as_str())
                    }
                })
            }
            SmtExpr::Apply(name, _args) => {
                // Create uninterpreted integer constant
                Ok(Int::new_const(format!("{}_result", name).as_str()))
            }
            SmtExpr::Select(arr, idx) => {
                let arr_name = match arr.as_ref() {
                    SmtExpr::Var(v) => v.smtlib_name(),
                    _ => Text::from("arr"),
                };
                let idx_str = idx.to_smtlib();
                Ok(Int::new_const(
                    format!("{}[{}]", arr_name, idx_str).as_str(),
                ))
            }
            SmtExpr::Store(_, _, _) => {
                // Store returns an array, not an integer
                Err(WPError::TypeError(Text::from(
                    "Array store expression cannot be used as integer",
                )))
            }
            SmtExpr::Ite(cond, then_e, else_e) => {
                let cond_z3 = self.formula_to_z3(cond)?;
                let then_z3 = self.expr_to_z3_int(then_e)?;
                let else_z3 = self.expr_to_z3_int(else_e)?;
                Ok(cond_z3.ite(&then_z3, &else_z3))
            }
            SmtExpr::Let(var, bound_expr, body) => {
                let substituted = body.substitute(var, bound_expr);
                self.expr_to_z3_int(&substituted)
            }
        }
    }

    /// Convert an SmtExpr to Z3 BV AST (bitvector-specific)
    ///
    /// Handles bitvector constants and operations, converting integer expressions
    /// to bitvectors when needed for mixed-mode operations.
    fn expr_to_z3_bv(&self, expr: &SmtExpr) -> Result<BV, WPError> {
        // Default bitvector width for conversions (64-bit)
        const DEFAULT_BV_WIDTH: u32 = 64;

        match expr {
            SmtExpr::Var(v) => {
                // Create a bitvector variable with default width
                Ok(BV::new_const(v.smtlib_name().as_str(), DEFAULT_BV_WIDTH))
            }
            SmtExpr::IntConst(n) => {
                // Convert integer to bitvector
                Ok(BV::from_i64(*n, DEFAULT_BV_WIDTH))
            }
            SmtExpr::BoolConst(b) => {
                // Convert boolean to 1-bit bitvector
                Ok(BV::from_u64(if *b { 1 } else { 0 }, 1))
            }
            SmtExpr::RealConst(r) => {
                // Truncate real to integer, then to bitvector
                Ok(BV::from_i64(*r as i64, DEFAULT_BV_WIDTH))
            }
            SmtExpr::BitVecConst(value, width) => {
                // Direct bitvector constant
                Ok(BV::from_u64(*value, *width))
            }
            SmtExpr::BinOp(op, left, right) => {
                let left_z3 = self.expr_to_z3_bv(left)?;
                let right_z3 = self.expr_to_z3_bv(right)?;

                // Ensure same width (extend if necessary)
                let (left_z3, right_z3) = if left_z3.get_size() != right_z3.get_size() {
                    let max_size = left_z3.get_size().max(right_z3.get_size());
                    let left_ext = if left_z3.get_size() < max_size {
                        left_z3.sign_ext(max_size - left_z3.get_size())
                    } else {
                        left_z3
                    };
                    let right_ext = if right_z3.get_size() < max_size {
                        right_z3.sign_ext(max_size - right_z3.get_size())
                    } else {
                        right_z3
                    };
                    (left_ext, right_ext)
                } else {
                    (left_z3, right_z3)
                };

                match op {
                    SmtBinOp::Add => Ok(left_z3.bvadd(&right_z3)),
                    SmtBinOp::Sub => Ok(left_z3.bvsub(&right_z3)),
                    SmtBinOp::Mul => Ok(left_z3.bvmul(&right_z3)),
                    SmtBinOp::Div => Ok(left_z3.bvsdiv(&right_z3)),
                    SmtBinOp::Mod => Ok(left_z3.bvsmod(&right_z3)),
                    SmtBinOp::Pow => {
                        // Power not directly supported for bitvectors
                        // Model as an uninterpreted function
                        let const_name = format!("bvpow({},{})", left_z3, right_z3);
                        Ok(BV::new_const(const_name.as_str(), left_z3.get_size()))
                    }
                    SmtBinOp::Select => {
                        // Select returns an element, not a bitvector operation
                        // Create a fresh bitvector for the result
                        let arr_name = match left.as_ref() {
                            SmtExpr::Var(v) => v.smtlib_name(),
                            _ => Text::from("arr"),
                        };
                        let idx_str = right.to_smtlib();
                        Ok(BV::new_const(
                            format!("{}[{}]", arr_name, idx_str).as_str(),
                            DEFAULT_BV_WIDTH,
                        ))
                    }
                }
            }
            SmtExpr::UnOp(op, inner) => {
                let inner_z3 = self.expr_to_z3_bv(inner)?;
                match op {
                    SmtUnOp::Neg => Ok(inner_z3.bvneg()),
                    SmtUnOp::Abs => {
                        // abs(x) = if x >= 0 then x else -x (for signed interpretation)
                        let zero = BV::from_i64(0, inner_z3.get_size());
                        let is_neg = inner_z3.bvslt(&zero);
                        let neg = inner_z3.bvneg();
                        // Use ite - need to convert to Dynamic first
                        let result = is_neg.ite(&neg, &inner_z3);
                        Ok(result)
                    }
                    SmtUnOp::Deref => {
                        // Dereference - return uninterpreted bitvector
                        Ok(BV::new_const(
                            format!("deref_{}", inner_z3).as_str(),
                            inner_z3.get_size(),
                        ))
                    }
                    SmtUnOp::Len => {
                        // Length - return bitvector representing length
                        Ok(BV::new_const(
                            format!("len_{}", inner_z3).as_str(),
                            DEFAULT_BV_WIDTH,
                        ))
                    }
                    SmtUnOp::GetVariantValue => {
                        // Get variant value - return uninterpreted bitvector
                        Ok(BV::new_const(
                            format!("variant_{}", inner_z3).as_str(),
                            inner_z3.get_size(),
                        ))
                    }
                }
            }
            SmtExpr::Apply(name, _args) => {
                // Uninterpreted function returning bitvector
                Ok(BV::new_const(
                    format!("{}_result", name).as_str(),
                    DEFAULT_BV_WIDTH,
                ))
            }
            SmtExpr::Select(arr, idx) => {
                let arr_name = match arr.as_ref() {
                    SmtExpr::Var(v) => v.smtlib_name(),
                    _ => Text::from("arr"),
                };
                let idx_str = idx.to_smtlib();
                Ok(BV::new_const(
                    format!("{}[{}]", arr_name, idx_str).as_str(),
                    DEFAULT_BV_WIDTH,
                ))
            }
            SmtExpr::Store(_, _, _) => {
                // Store returns an array, not a bitvector
                Err(WPError::TypeError(Text::from(
                    "Array store expression cannot be used as bitvector",
                )))
            }
            SmtExpr::Ite(cond, then_e, else_e) => {
                let cond_z3 = self.formula_to_z3(cond)?;
                let then_z3 = self.expr_to_z3_bv(then_e)?;
                let else_z3 = self.expr_to_z3_bv(else_e)?;

                // Ensure same width for ite branches
                let (then_z3, else_z3) = if then_z3.get_size() != else_z3.get_size() {
                    let max_size = then_z3.get_size().max(else_z3.get_size());
                    let then_ext = if then_z3.get_size() < max_size {
                        then_z3.sign_ext(max_size - then_z3.get_size())
                    } else {
                        then_z3
                    };
                    let else_ext = if else_z3.get_size() < max_size {
                        else_z3.sign_ext(max_size - else_z3.get_size())
                    } else {
                        else_z3
                    };
                    (then_ext, else_ext)
                } else {
                    (then_z3, else_z3)
                };

                Ok(cond_z3.ite(&then_z3, &else_z3))
            }
            SmtExpr::Let(var, bound_expr, body) => {
                let substituted = body.substitute(var, bound_expr);
                self.expr_to_z3_bv(&substituted)
            }
        }
    }

    /// Convert array select operation to Z3
    ///
    /// Handles both proper Z3 array theory operations and fallback to
    /// uninterpreted functions when array type cannot be determined.
    fn expr_to_z3_select(&self, arr: &SmtExpr, idx: &SmtExpr) -> Result<Dynamic, WPError> {
        // Try to get array variable name for proper Z3 array handling
        let arr_name = match arr {
            SmtExpr::Var(v) => v.smtlib_name(),
            _ => {
                // For complex expressions, use uninterpreted function fallback
                let arr_z3 = self.expr_to_z3(arr)?;
                let idx_z3 = self.expr_to_z3(idx)?;
                let const_name = format!("select({},{})", arr_z3, idx_z3);
                return Ok(Dynamic::from_ast(&Int::new_const(const_name.as_str())));
            }
        };

        // Create a Z3 array constant with Int domain and Int range (default)
        let z3_arr = Array::new_const(arr_name.as_str(), &Sort::int(), &Sort::int());
        let idx_z3 = self.expr_to_z3_int(idx)?;

        // Use Z3's native array select operation
        Ok(z3_arr.select(&idx_z3))
    }

    /// Convert array store operation to Z3
    ///
    /// Returns a new array with the element at the given index updated.
    fn expr_to_z3_store(
        &self,
        arr: &SmtExpr,
        idx: &SmtExpr,
        val: &SmtExpr,
    ) -> Result<Dynamic, WPError> {
        // Try to get array variable name for proper Z3 array handling
        let arr_name = match arr {
            SmtExpr::Var(v) => v.smtlib_name(),
            SmtExpr::Store(inner_arr, _, _) => {
                // Nested store - recursively convert
                let inner_z3 = self.expr_to_z3_store(inner_arr, idx, val)?;
                let idx_z3 = self.expr_to_z3_int(idx)?;
                let val_z3 = self.expr_to_z3_int(val)?;

                // Need to cast to Array - fallback to uninterpreted if can't
                let const_name = format!("store({},{},{})", inner_z3, idx_z3, val_z3);
                return Ok(Dynamic::from_ast(&Int::new_const(const_name.as_str())));
            }
            _ => {
                // For complex expressions, use uninterpreted function fallback
                let arr_z3 = self.expr_to_z3(arr)?;
                let idx_z3 = self.expr_to_z3(idx)?;
                let val_z3 = self.expr_to_z3(val)?;
                let const_name = format!("store({},{},{})", arr_z3, idx_z3, val_z3);
                return Ok(Dynamic::from_ast(&Int::new_const(const_name.as_str())));
            }
        };

        // Create a Z3 array constant with Int domain and Int range (default)
        let z3_arr = Array::new_const(arr_name.as_str(), &Sort::int(), &Sort::int());
        let idx_z3 = self.expr_to_z3_int(idx)?;
        let val_z3 = self.expr_to_z3_int(val)?;

        // Use Z3's native array store operation
        Ok(Dynamic::from_ast(&z3_arr.store(&idx_z3, &val_z3)))
    }

    /// Convert a tuple field access to Z3
    ///
    /// Tuples are modeled as arrays indexed by field position.
    fn expr_to_z3_tuple_access(&self, tuple: &SmtExpr, field_idx: i64) -> Result<Dynamic, WPError> {
        let tuple_name = match tuple {
            SmtExpr::Var(v) => v.smtlib_name(),
            _ => {
                // For complex expressions, create uninterpreted function
                let tuple_z3 = self.expr_to_z3(tuple)?;
                let const_name = format!("tuple_get({},{})", tuple_z3, field_idx);
                return Ok(Dynamic::from_ast(&Int::new_const(const_name.as_str())));
            }
        };

        // Model tuple as array and use select
        let tuple_arr = Array::new_const(
            format!("{}_tuple", tuple_name).as_str(),
            &Sort::int(),
            &Sort::int(),
        );
        let idx = Int::from_i64(field_idx);
        Ok(tuple_arr.select(&idx))
    }

    /// Generate VCs for a function with contract
    pub fn generate_function_vcs(
        &self,
        func: &FunctionContract,
    ) -> Result<List<VerificationCondition>, WPError> {
        let mut vcs = Vec::new();

        // Convert function body to command
        let command = self.function_to_command(&func.body)?;

        // Main correctness VC: requires => wp(body, ensures)
        let triple = HoareTriple::new(func.requires.clone(), command, func.ensures.clone());

        let vc = self.generate_vc(&triple)?;
        vcs.push(vc);

        Ok(vcs.into())
    }

    /// Convert a function body to a command
    fn function_to_command(&self, body: &FunctionBody) -> Result<Command, WPError> {
        match body {
            FunctionBody::Expr(expr) => self.expr_to_command(expr),
            FunctionBody::Block(stmts) => {
                let cmds: Result<List<Command>, WPError> =
                    stmts.iter().map(|s| self.stmt_to_command(s)).collect();
                Ok(Command::Block(cmds?))
            }
        }
    }

    /// Convert an expression to a command
    ///
    /// This is the production implementation that handles all expression types
    /// and converts them to Hoare logic commands.
    ///
    /// ## Expression Categories
    ///
    /// 1. **Literals**: Produce skip (no side effects)
    /// 2. **Variables**: Produce skip (no side effects, just lookup)
    /// 3. **Binary/Unary ops**: Produce skip (pure computations)
    /// 4. **Assignments**: Produce Assign command
    /// 5. **If expressions**: Produce If command
    /// 6. **While/For loops**: Produce While/For command
    /// 7. **Function calls**: Produce Call command
    /// 8. **Blocks**: Produce Block command
    fn expr_to_command(&self, expr: &Expr) -> Result<Command, WPError> {
        match &expr.kind {
            // Pure expressions with no side effects
            ExprKind::Literal(_) => Ok(Command::Skip),
            ExprKind::Path(_) => Ok(Command::Skip),
            ExprKind::Binary { .. } => Ok(Command::Skip),
            ExprKind::Unary { .. } => Ok(Command::Skip),
            ExprKind::Tuple(_) => Ok(Command::Skip),
            ExprKind::Array(_) => Ok(Command::Skip),
            ExprKind::Record { .. } => Ok(Command::Skip),
            ExprKind::Range { .. } => Ok(Command::Skip),

            // If expression
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Extract the first condition expression from IfCondition
                let cond_formula = if let Some(first_cond) = condition.conditions.first() {
                    match first_cond {
                        verum_ast::ConditionKind::Expr(expr) => self.expr_to_formula(expr)?,
                        verum_ast::ConditionKind::Let { .. } => {
                            // Let conditions are more complex - approximate as true
                            Formula::True
                        }
                    }
                } else {
                    Formula::True
                };
                let then_cmd = self.block_to_command(then_branch)?;
                let else_cmd: Maybe<Heap<Command>> = match else_branch.as_ref() {
                    Some(else_block) => {
                        Maybe::Some(Heap::new(self.expr_to_command(else_block.as_ref())?))
                    }
                    None => Maybe::None,
                };
                Ok(Command::If {
                    condition: cond_formula,
                    then_branch: Heap::new(then_cmd),
                    else_branch: else_cmd,
                })
            }

            // Match expression (convert to nested if-else)
            ExprKind::Match { expr, arms } => self.match_to_command(expr.as_ref(), arms),

            // While loop
            ExprKind::While {
                condition,
                body,
                label: _,
                invariants: _,
                decreases: _,
            } => {
                let cond_formula = self.expr_to_formula(condition.as_ref())?;
                let body_cmd = self.block_to_command(body)?;
                // Use true as default invariant (to be filled by contract)
                Ok(Command::While {
                    condition: cond_formula,
                    invariant: Formula::True,
                    body: Heap::new(body_cmd),
                    decreases: Maybe::None,
                    lexicographic_decreases: Maybe::None,
                })
            }

            // For loop
            ExprKind::For {
                pattern,
                iter,
                body,
                label: _,
                invariants: _,
                decreases: _,
            } => self.for_to_command(pattern, iter, body),

            // Loop (infinite loop until break)
            ExprKind::Loop {
                body,
                label: _,
                invariants: _,
            } => {
                let body_cmd = self.block_to_command(body)?;
                Ok(Command::While {
                    condition: Formula::True,
                    invariant: Formula::True,
                    body: Heap::new(body_cmd),
                    decreases: Maybe::None,
                    lexicographic_decreases: Maybe::None,
                })
            }

            // Block expression
            ExprKind::Block(block) => self.block_to_command(block),

            // Assignment is handled via Binary with BinOp::Assign
            // Check for assignment in Binary handling below

            // Function call
            ExprKind::Call { func, args, .. } => {
                let func_name = self.expr_to_func_name(func)?;
                let arg_exprs: Result<List<SmtExpr>, WPError> =
                    args.iter().map(|a| self.expr_to_smt(a)).collect();
                Ok(Command::Call {
                    result: Maybe::None, // No result assignment in expression position
                    function: func_name,
                    args: arg_exprs?,
                    requires: Formula::True, // Default - actual contract from context
                    ensures: Formula::True,
                })
            }

            // Method call (desugars to function call)
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                let mut all_args = vec![self.expr_to_smt(receiver)?];
                for arg in args.iter() {
                    all_args.push(self.expr_to_smt(arg)?);
                }
                Ok(Command::Call {
                    result: Maybe::None,
                    function: Text::from(method.name.as_str()),
                    args: all_args.into_iter().collect(),
                    requires: Formula::True,
                    ensures: Formula::True,
                })
            }

            // Return expression (for now, just evaluate the value)
            ExprKind::Return(maybe_expr) => {
                match maybe_expr {
                    Maybe::Some(return_expr) => {
                        // Create an assignment to the special result variable
                        let result_var = Variable::new("result");
                        let result_smt = self.expr_to_smt(return_expr)?;
                        Ok(Command::Assign {
                            var: result_var,
                            expr: result_smt,
                        })
                    }
                    Maybe::None => Ok(Command::Skip),
                }
            }

            // Break/Continue (control flow - represented as skip for WP)
            ExprKind::Break { .. } => Ok(Command::Skip),
            ExprKind::Continue { .. } => Ok(Command::Skip),

            // Try expression (handle errors)
            ExprKind::Try(inner) => self.expr_to_command(inner),

            // Await expression (async - handle as function call)
            ExprKind::Await(inner) => self.expr_to_command(inner),

            // Field access (pure)
            ExprKind::Field { .. } => Ok(Command::Skip),
            ExprKind::OptionalChain { .. } => Ok(Command::Skip),

            // Index access (may have effects if index method does)
            ExprKind::Index { .. } => Ok(Command::Skip),

            // Closure (pure - creates a value)
            ExprKind::Closure { .. } => Ok(Command::Skip),

            // Reference operations are handled via UnOp

            // Type cast (pure)
            ExprKind::Cast { .. } => Ok(Command::Skip),

            // Catch-all for any other expression kinds:
            // - Error expressions (from parse errors)
            // - Future expression kinds added to the AST
            // These are treated as skip (no-op) since they don't have
            // a well-defined Hoare logic semantics or represent errors.
            _ => Ok(Command::Skip),
        }
    }

    /// Convert a statement to a command
    ///
    /// This is the production implementation that handles all statement types
    /// and converts them to Hoare logic commands.
    fn stmt_to_command(&self, stmt: &Stmt) -> Result<Command, WPError> {
        match &stmt.kind {
            // Let binding: let x = e
            StmtKind::Let {
                pattern,
                ty: _,
                value,
            } => {
                match value {
                    Maybe::Some(init_expr) => {
                        let var = self.pattern_to_variable(pattern)?;
                        let val_smt = self.expr_to_smt(init_expr)?;
                        Ok(Command::Assign { var, expr: val_smt })
                    }
                    Maybe::None => {
                        // Uninitialized variable - havoc
                        let var = self.pattern_to_variable(pattern)?;
                        Ok(Command::Havoc(var))
                    }
                }
            }

            // Let-else: let pattern = expr else { diverge }
            // Pattern matching in Hoare logic: model as conditional binding.
            // If pattern matches value, bind variables; otherwise take else branch.
            StmtKind::LetElse {
                pattern,
                ty: _,
                value,
                else_block,
            } => {
                // Model as: if matches(pattern, value) then bind else diverge
                let var = self.pattern_to_variable(pattern)?;
                let val_smt = self.expr_to_smt(value)?;
                let else_cmd = self.block_to_command(else_block)?;

                // Generate pattern match condition from the pattern structure
                let pattern_matches = self.pattern_to_condition(pattern, &val_smt)?;

                // Generate variable bindings extracted from the pattern
                let bindings = self.extract_pattern_bindings(pattern, &val_smt)?;

                // Build the then branch: sequence of bindings followed by implicit continuation
                let then_cmds: List<Command> = bindings
                    .into_iter()
                    .map(|(name, expr)| Command::Assign {
                        var: Variable::new(name.as_str()),
                        expr,
                    })
                    .collect();

                let then_branch = if then_cmds.is_empty() {
                    Command::Assign { var, expr: val_smt }
                } else {
                    let mut cmds = then_cmds;
                    cmds.push(Command::Assign {
                        var: var.clone(),
                        expr: val_smt.clone(),
                    });
                    Command::Block(cmds)
                };

                Ok(Command::If {
                    condition: pattern_matches,
                    then_branch: Heap::new(then_branch),
                    else_branch: Maybe::Some(Heap::new(else_cmd)),
                })
            }

            // Expression statement
            StmtKind::Expr { expr, has_semi: _ } => self.expr_to_command(expr),

            // Item declaration (function, type, etc.) - skip for verification
            StmtKind::Item(_) => Ok(Command::Skip),

            // Defer statement - skip for now (deferred cleanup)
            StmtKind::Defer(_) => Ok(Command::Skip),

            // Errdefer statement - skip for now (error-path-only cleanup)
            // For verification, errdefer only affects error paths
            StmtKind::Errdefer(_) => Ok(Command::Skip),

            // Provide statement (context injection)
            StmtKind::Provide {
                context: _,
                alias: _,
                value: _,
            } => Ok(Command::Skip),

            // Provide scope - skip for verification
            StmtKind::ProvideScope {
                context: _,
                alias: _,
                value: _,
                block: _,
            } => Ok(Command::Skip),

            // Empty statement
            StmtKind::Empty => Ok(Command::Skip),
        }
    }

    /// Convert a block to a command (sequence of statements)
    fn block_to_command(&self, block: &Block) -> Result<Command, WPError> {
        if block.stmts.is_empty() {
            return Ok(Command::Skip);
        }

        let cmds: Result<List<Command>, WPError> = block
            .stmts
            .iter()
            .map(|s| self.stmt_to_command(s))
            .collect();
        Ok(Command::Block(cmds?))
    }

    /// Convert a match expression to nested if-else commands
    fn match_to_command(
        &self,
        scrutinee: &Expr,
        arms: &List<verum_ast::MatchArm>,
    ) -> Result<Command, WPError> {
        if arms.is_empty() {
            return Ok(Command::Skip);
        }

        // Build nested if-else from arms
        let scrutinee_smt = self.expr_to_smt(scrutinee)?;
        self.arms_to_command(&scrutinee_smt, arms.iter().collect::<Vec<_>>().as_slice())
    }

    /// Convert match arms to nested if-else
    fn arms_to_command(
        &self,
        scrutinee: &SmtExpr,
        arms: &[&verum_ast::MatchArm],
    ) -> Result<Command, WPError> {
        if arms.is_empty() {
            return Ok(Command::Skip);
        }

        let first = arms[0];
        let rest = &arms[1..];

        // Convert pattern to condition
        let pattern_cond = self.pattern_to_condition(&first.pattern, scrutinee)?;

        // Convert body
        let body_cmd = self.expr_to_command(&first.body)?;

        if rest.is_empty() {
            // Last arm - just execute body (no else)
            Ok(Command::If {
                condition: pattern_cond,
                then_branch: Heap::new(body_cmd),
                else_branch: Maybe::None,
            })
        } else {
            // More arms - build else branch recursively
            let else_cmd = self.arms_to_command(scrutinee, rest)?;
            Ok(Command::If {
                condition: pattern_cond,
                then_branch: Heap::new(body_cmd),
                else_branch: Maybe::Some(Heap::new(else_cmd)),
            })
        }
    }

    /// Convert a for loop to a while loop command
    fn for_to_command(
        &self,
        pattern: &Pattern,
        iterable: &Expr,
        body: &Block,
    ) -> Result<Command, WPError> {
        // Extract loop variable from pattern
        let var = self.pattern_to_variable(pattern)?;

        // Try to extract range bounds from iterable
        let (start, end) = self.extract_range_bounds(iterable)?;

        // Convert body
        let body_cmd = self.block_to_command(body)?;

        Ok(Command::For {
            var,
            start,
            end,
            invariant: Formula::True,
            body: Heap::new(body_cmd),
        })
    }

    /// Extract range bounds from an iterable expression
    fn extract_range_bounds(&self, iterable: &Expr) -> Result<(SmtExpr, SmtExpr), WPError> {
        match &iterable.kind {
            ExprKind::Range {
                start,
                end,
                inclusive,
            } => {
                let start_smt = match start {
                    Maybe::Some(s) => self.expr_to_smt(s)?,
                    Maybe::None => SmtExpr::int(0),
                };
                let end_smt = match end {
                    Maybe::Some(e) => {
                        let e_smt = self.expr_to_smt(e)?;
                        if *inclusive {
                            // Inclusive end: use end + 1
                            SmtExpr::add(e_smt, SmtExpr::int(1))
                        } else {
                            e_smt
                        }
                    }
                    Maybe::None => {
                        // Unbounded range: use widening with symbolic upper bound
                        // This models unbounded iteration with a symbolic limit
                        // The verification will need an explicit loop invariant to be sound
                        SmtExpr::var("unbounded_iter_end")
                    }
                };
                Ok((start_smt, end_smt))
            }
            ExprKind::Call { .. } => {
                // Iterator from function call (e.g., iter(), into_iter())
                // Use symbolic bounds representing the iterator's length
                let iter_name = format!("iter_{}", self.fresh_id());
                Ok((SmtExpr::int(0), SmtExpr::var(format!("{}_len", iter_name))))
            }
            ExprKind::Path(path) => {
                // Variable holding an iterable (e.g., array, list)
                // Model as iteration from 0 to length
                let var_name = path
                    .segments
                    .first()
                    .map(|s| Text::from(path_segment_to_str(s)))
                    .unwrap_or_else(|| Text::from("unknown"));
                let len_expr =
                    SmtExpr::UnOp(SmtUnOp::Len, Box::new(SmtExpr::var(var_name.to_string())));
                Ok((SmtExpr::int(0), len_expr))
            }
            _ => {
                // Non-range iterable - use symbolic bounds
                Ok((SmtExpr::int(0), SmtExpr::var("iter_end")))
            }
        }
    }

    /// Generate a fresh unique identifier for symbolic names
    fn fresh_id(&self) -> u64 {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        COUNTER.fetch_add(1, Ordering::SeqCst)
    }

    /// Convert an expression to a formula (for conditions)
    fn expr_to_formula(&self, expr: &Expr) -> Result<Formula, WPError> {
        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                verum_ast::LiteralKind::Bool(true) => Ok(Formula::True),
                verum_ast::LiteralKind::Bool(false) => Ok(Formula::False),
                _ => Ok(Formula::Predicate(
                    Text::from("literal"),
                    vec![self.expr_to_smt(expr)?].into(),
                )),
            },
            ExprKind::Binary { op, left, right } => {
                match op {
                    BinOp::And => {
                        let l = self.expr_to_formula(left)?;
                        let r = self.expr_to_formula(right)?;
                        Ok(Formula::and(vec![l, r]))
                    }
                    BinOp::Or => {
                        let l = self.expr_to_formula(left)?;
                        let r = self.expr_to_formula(right)?;
                        Ok(Formula::or(vec![l, r]))
                    }
                    BinOp::Eq => {
                        let l = self.expr_to_smt(left)?;
                        let r = self.expr_to_smt(right)?;
                        Ok(Formula::Eq(Box::new(l), Box::new(r)))
                    }
                    BinOp::Ne => {
                        let l = self.expr_to_smt(left)?;
                        let r = self.expr_to_smt(right)?;
                        Ok(Formula::Not(Box::new(Formula::Eq(
                            Box::new(l),
                            Box::new(r),
                        ))))
                    }
                    BinOp::Lt => Ok(Formula::lt(
                        self.expr_to_smt(left)?,
                        self.expr_to_smt(right)?,
                    )),
                    BinOp::Le => Ok(Formula::le(
                        self.expr_to_smt(left)?,
                        self.expr_to_smt(right)?,
                    )),
                    BinOp::Gt => Ok(Formula::gt(
                        self.expr_to_smt(left)?,
                        self.expr_to_smt(right)?,
                    )),
                    BinOp::Ge => Ok(Formula::ge(
                        self.expr_to_smt(left)?,
                        self.expr_to_smt(right)?,
                    )),
                    _ => {
                        // Other binary ops: treat as boolean expression
                        Ok(Formula::Predicate(
                            Text::from("expr"),
                            vec![self.expr_to_smt(expr)?].into(),
                        ))
                    }
                }
            }
            ExprKind::Unary { op, expr: inner } => match op {
                UnOp::Not => {
                    let inner_formula = self.expr_to_formula(inner)?;
                    Ok(Formula::Not(Box::new(inner_formula)))
                }
                _ => Ok(Formula::Predicate(
                    Text::from("expr"),
                    vec![self.expr_to_smt(expr)?].into(),
                )),
            },
            ExprKind::Path(path) => {
                // Variable reference as boolean
                let var_name = path
                    .segments
                    .first()
                    .map(|s| Text::from(path_segment_to_str(s)))
                    .unwrap_or_else(|| Text::from("unknown"));
                Ok(Formula::Var(Variable::new(var_name.to_string())))
            }
            _ => {
                // Treat other expressions as predicates
                Ok(Formula::Predicate(
                    Text::from("expr"),
                    vec![self.expr_to_smt(expr)?].into(),
                ))
            }
        }
    }

    /// Convert an expression to an SMT expression
    fn expr_to_smt(&self, expr: &Expr) -> Result<SmtExpr, WPError> {
        match &expr.kind {
            ExprKind::Literal(lit) => {
                match &lit.kind {
                    verum_ast::LiteralKind::Int(n) => Ok(SmtExpr::int(n.value as i64)),
                    verum_ast::LiteralKind::Float(f) => Ok(SmtExpr::real(f.value)),
                    verum_ast::LiteralKind::Bool(b) => Ok(SmtExpr::bool(*b)),
                    verum_ast::LiteralKind::Text(s) => {
                        Ok(SmtExpr::Var(Variable::new(format!("str_{:?}", s))))
                    }
                    verum_ast::LiteralKind::Char(c) => Ok(SmtExpr::int(*c as i64)),
                    _ => Ok(SmtExpr::int(0)), // Other literals as 0
                }
            }
            ExprKind::Path(path) => {
                let var_name = path
                    .segments
                    .first()
                    .map(|s| Text::from(path_segment_to_str(s)))
                    .unwrap_or_else(|| Text::from("unknown"));
                Ok(SmtExpr::var(var_name.to_string()))
            }
            ExprKind::Binary { op, left, right } => {
                let l = self.expr_to_smt(left)?;
                let r = self.expr_to_smt(right)?;
                Ok(self.apply_binop(*op, l, r))
            }
            ExprKind::Unary { op, expr: inner } => {
                let inner_smt = self.expr_to_smt(inner)?;
                match op {
                    UnOp::Neg => Ok(SmtExpr::neg(inner_smt)),
                    UnOp::Not => {
                        // Represent logical NOT as an application
                        Ok(SmtExpr::Apply(Text::from("not"), vec![inner_smt].into()))
                    }
                    _ => Ok(inner_smt), // Other unary ops - just return inner
                }
            }
            ExprKind::Field { expr: base, field } => {
                let base_smt = self.expr_to_smt(base)?;
                // Create a field access expression
                Ok(SmtExpr::Select(
                    Box::new(base_smt),
                    Box::new(SmtExpr::Var(Variable::new(field.name.to_string()))),
                ))
            }
            ExprKind::Index { expr: base, index } => {
                let base_smt = self.expr_to_smt(base)?;
                let index_smt = self.expr_to_smt(index)?;
                Ok(SmtExpr::Select(Box::new(base_smt), Box::new(index_smt)))
            }
            ExprKind::Call { func, args, .. } => {
                let func_name = self.expr_to_func_name(func)?;
                let arg_smts: Result<Vec<SmtExpr>, WPError> =
                    args.iter().map(|a| self.expr_to_smt(a)).collect();
                Ok(SmtExpr::Apply(func_name, arg_smts?.into()))
            }
            _ => {
                // For complex expressions, create a fresh variable
                Ok(SmtExpr::var(format!("expr_{:p}", expr as *const _)))
            }
        }
    }

    /// Apply a binary operation to two SMT expressions
    fn apply_binop(&self, op: BinOp, left: SmtExpr, right: SmtExpr) -> SmtExpr {
        match op {
            BinOp::Add => SmtExpr::add(left, right),
            BinOp::Sub => SmtExpr::sub(left, right),
            BinOp::Mul => SmtExpr::mul(left, right),
            BinOp::Div => SmtExpr::BinOp(SmtBinOp::Div, Box::new(left), Box::new(right)),
            BinOp::Rem => SmtExpr::BinOp(SmtBinOp::Mod, Box::new(left), Box::new(right)),
            BinOp::Eq => SmtExpr::Apply(Text::from("="), vec![left, right].into()),
            BinOp::Ne => SmtExpr::Apply(Text::from("distinct"), vec![left, right].into()),
            BinOp::Lt => SmtExpr::Apply(Text::from("<"), vec![left, right].into()),
            BinOp::Le => SmtExpr::Apply(Text::from("<="), vec![left, right].into()),
            BinOp::Gt => SmtExpr::Apply(Text::from(">"), vec![left, right].into()),
            BinOp::Ge => SmtExpr::Apply(Text::from(">="), vec![left, right].into()),
            BinOp::And => SmtExpr::Apply(Text::from("and"), vec![left, right].into()),
            BinOp::Or => SmtExpr::Apply(Text::from("or"), vec![left, right].into()),
            _ => SmtExpr::Apply(Text::from(format!("{:?}", op)), vec![left, right].into()),
        }
    }

    /// Convert an expression to a variable (for assignment targets)
    ///
    /// Handles complex assignment targets like:
    /// - Simple variables: `x = ...`
    /// - Field access: `obj.field = ...` (modeled as obj_field)
    /// - Array index: `arr[i] = ...` (modeled as arr_i)
    /// - Dereference: `*ptr = ...` (modeled as deref_ptr)
    fn expr_to_variable(&self, expr: &Expr) -> Result<Variable, WPError> {
        match &expr.kind {
            ExprKind::Path(path) => {
                let var_name = path
                    .segments
                    .first()
                    .map(|s| Text::from(path_segment_to_str(s)))
                    .unwrap_or_else(|| Text::from("unknown"));
                Ok(Variable::new(var_name.to_string()))
            }
            ExprKind::Field { expr: base, field } => {
                // Field access: model as base_field composite variable
                let base_var = self.expr_to_variable(base)?;
                let composite_name = format!("{}.{}", base_var.name, field.name);
                Ok(Variable::new(composite_name))
            }
            ExprKind::Index { expr: base, index } => {
                // Array index: model as base[index] composite variable
                let base_var = self.expr_to_variable(base)?;
                let index_smt = self.expr_to_smt(index)?;
                let composite_name = format!("{}[{}]", base_var.name, index_smt.to_smtlib());
                Ok(Variable::new(composite_name))
            }
            ExprKind::Unary {
                op: UnOp::Deref,
                expr: inner,
            } => {
                // Dereference: model as deref(ptr) composite variable
                let inner_var = self.expr_to_variable(inner)?;
                let composite_name = format!("deref({})", inner_var.name);
                Ok(Variable::new(composite_name))
            }
            ExprKind::Paren(inner) => {
                // Parenthesized expression: unwrap
                self.expr_to_variable(inner)
            }
            _ => {
                // For other complex expressions, generate a fresh variable
                // This is a conservative approach that models the assignment target
                // as an opaque location
                let fresh_name = format!("target_{}", self.fresh_id());
                Ok(Variable::new(fresh_name))
            }
        }
    }

    /// Convert a pattern to a variable
    ///
    /// Handles complex patterns for let bindings:
    /// - Simple identifiers: `let x = ...`
    /// - Wildcards: `let _ = ...` (no binding)
    /// - Tuples: `let (a, b) = ...` (returns first element's variable)
    /// - References: `let &x = ...` (unwraps reference)
    fn pattern_to_variable(&self, pattern: &Pattern) -> Result<Variable, WPError> {
        match &pattern.kind {
            PatternKind::Ident { name, .. } => Ok(Variable::new(name.name.to_string())),
            PatternKind::Wildcard => {
                // Wildcard binds nothing, use a fresh unused variable
                Ok(Variable::new(format!("_unused_{}", self.fresh_id())))
            }
            PatternKind::Tuple(patterns) => {
                // For tuple patterns, we handle destructuring
                // Return the first bound variable for simple cases
                // More complex handling would require multiple variables
                if let Some(first) = patterns.first() {
                    self.pattern_to_variable(first)
                } else {
                    // Empty tuple - use unit variable
                    Ok(Variable::new("unit"))
                }
            }
            PatternKind::Reference { inner, .. } => {
                // Reference pattern: unwrap and get inner variable
                self.pattern_to_variable(inner)
            }
            PatternKind::Paren(inner) => {
                // Parenthesized pattern: unwrap
                self.pattern_to_variable(inner)
            }
            PatternKind::Record { path, fields, .. } => {
                // Record pattern: use the type name as a prefix
                let type_name = path
                    .segments
                    .last()
                    .map(|s| Text::from(path_segment_to_str(s)))
                    .unwrap_or_else(|| Text::from("record"));
                // Return composite variable for the whole record
                Ok(Variable::new(format!("{}_instance", type_name)))
            }
            PatternKind::Variant { path, .. } => {
                // Variant pattern: use the variant name
                let variant_name = path
                    .segments
                    .last()
                    .map(|s| Text::from(path_segment_to_str(s)))
                    .unwrap_or_else(|| Text::from("variant"));
                Ok(Variable::new(format!("{}_value", variant_name)))
            }
            PatternKind::Literal(lit) => {
                // Literal patterns don't bind, but we still need a variable
                // for the value being matched
                let lit_str = format!("{:?}", lit);
                Ok(Variable::new(format!("match_{}", lit_str)))
            }
            PatternKind::Or(patterns) => {
                // Or pattern: use first alternative's variable
                if let Some(first) = patterns.first() {
                    self.pattern_to_variable(first)
                } else {
                    Ok(Variable::new(format!("or_pattern_{}", self.fresh_id())))
                }
            }
            PatternKind::Range { .. } => {
                // Range pattern: use a fresh variable for the matched value
                Ok(Variable::new(format!("range_match_{}", self.fresh_id())))
            }
            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                // Slice pattern: return first bound variable if any
                if let Some(first) = before.first() {
                    self.pattern_to_variable(first)
                } else if let Maybe::Some(rest_pat) = rest {
                    self.pattern_to_variable(rest_pat)
                } else if let Some(first) = after.first() {
                    self.pattern_to_variable(first)
                } else {
                    Ok(Variable::new(format!("slice_{}", self.fresh_id())))
                }
            }
            PatternKind::Array(patterns) => {
                // Array pattern: return first bound variable
                if let Some(first) = patterns.first() {
                    self.pattern_to_variable(first)
                } else {
                    Ok(Variable::new(format!("array_{}", self.fresh_id())))
                }
            }
            PatternKind::Rest => {
                // Rest pattern: represents remaining elements
                Ok(Variable::new(format!("rest_{}", self.fresh_id())))
            }            PatternKind::View { .. } => {
                // View pattern: use the inner pattern
                Ok(Variable::new(format!("view_{}", self.fresh_id())))
            }
            PatternKind::Active { name, .. } => {
                // Active pattern: use the pattern name
                Ok(Variable::new(format!("active_{}_{}", name.name, self.fresh_id())))
            }
            PatternKind::And(patterns) => {
                // And pattern: use first pattern's variable
                if let Some(first) = patterns.first() {
                    self.pattern_to_variable(first)
                } else {
                    Ok(Variable::new(format!("and_pattern_{}", self.fresh_id())))
                }
            }
            PatternKind::TypeTest { binding, .. } => {
                // Type test pattern: binding is Type
                // Returns a variable for the binding name (with narrowed type)
                Ok(Variable::new(binding.name.to_string()))
            }
            PatternKind::Stream { head_patterns, rest } => {
                // Stream pattern: stream[first, second, ...rest]
                // Return variable from first head pattern or rest binding
                if let Some(first) = head_patterns.first() {
                    self.pattern_to_variable(first)
                } else if let Maybe::Some(rest_ident) = rest {
                    Ok(Variable::new(rest_ident.name.to_string()))
                } else {
                    Ok(Variable::new(format!("stream_{}", self.fresh_id())))
                }
            }
            PatternKind::Guard { pattern, .. } => {
                // Guard pattern: (pattern if expr)
                // Spec: Rust RFC 3637 - Guard Patterns
                // Return the variable from the inner pattern
                self.pattern_to_variable(pattern)
            }
            PatternKind::Cons { head, .. } => {
                self.pattern_to_variable(head)
            }
        }
    }

    /// Convert a pattern to a condition formula for matching
    ///
    /// This implements pattern matching semantics by generating conditions
    /// that determine whether a value matches a pattern structure.
    ///
    /// Generates conditions for pattern matching: wildcard always matches,
    /// literal patterns check equality, variant patterns check tag and bind fields,
    /// struct patterns check all field matches, tuple patterns check component matches.
    fn pattern_to_condition(
        &self,
        pattern: &Pattern,
        scrutinee: &SmtExpr,
    ) -> Result<Formula, WPError> {
        match &pattern.kind {
            // Wildcard always matches
            PatternKind::Wildcard => Ok(Formula::True),

            // Identifier binding always matches (captures any value)
            PatternKind::Ident { .. } => Ok(Formula::True),

            // Literal pattern: scrutinee == literal_value
            PatternKind::Literal(lit) => {
                let lit_smt = self.literal_to_smt(lit)?;
                Ok(Formula::Eq(Box::new(scrutinee.clone()), Box::new(lit_smt)))
            }

            // Tuple pattern: match each element
            PatternKind::Tuple(patterns) => {
                let mut conditions = List::new();
                for (i, pat) in patterns.iter().enumerate() {
                    // Access tuple element: tuple_get(scrutinee, i)
                    let elem_access = SmtExpr::Apply(
                        Text::from("tuple_get"),
                        vec![scrutinee.clone(), SmtExpr::int(i as i64)].into(),
                    );
                    let cond = self.pattern_to_condition(pat, &elem_access)?;
                    conditions.push(cond);
                }
                Ok(Formula::and(conditions))
            }

            // Struct/Record pattern: check type and field values
            PatternKind::Record { path, fields, .. } => {
                let constructor_name = path
                    .segments
                    .last()
                    .map(|s| Text::from(path_segment_to_str(s)))
                    .unwrap_or_else(|| Text::from("unknown"));

                // Type discriminant check: is_Constructor(scrutinee)
                let type_check = Formula::Predicate(
                    Text::from("is"),
                    vec![
                        scrutinee.clone(),
                        SmtExpr::var(constructor_name.to_string()),
                    ]
                    .into(),
                );

                // Field pattern checks
                let mut conditions = vec![type_check];
                for field in fields.iter() {
                    let field_name = field.name.as_str();
                    let field_access = SmtExpr::Apply(
                        Text::from(format!("field_{}", field_name)),
                        vec![scrutinee.clone()].into(),
                    );
                    // Handle optional pattern (shorthand notation like { x } vs { x: pat })
                    if let Some(pat) = &field.pattern {
                        let field_cond = self.pattern_to_condition(pat, &field_access)?;
                        conditions.push(field_cond);
                    }
                }

                Ok(Formula::and(conditions))
            }

            // Variant/enum pattern: check discriminant and inner pattern
            PatternKind::Variant { path, data } => {
                let constructor_name = path
                    .segments
                    .last()
                    .map(|s| Text::from(path_segment_to_str(s)))
                    .unwrap_or_else(|| Text::from("unknown"));

                // Discriminant check: is(scrutinee, VariantName)
                let variant_check = Formula::Predicate(
                    Text::from("is"),
                    vec![
                        scrutinee.clone(),
                        SmtExpr::var(constructor_name.to_string()),
                    ]
                    .into(),
                );

                // Inner pattern check if present
                match data {
                    Maybe::Some(variant_data) => {
                        // Extract inner value from variant for pattern matching
                        let inner_access =
                            SmtExpr::UnOp(SmtUnOp::GetVariantValue, Box::new(scrutinee.clone()));

                        let mut conditions = vec![variant_check];

                        match variant_data {
                            verum_ast::pattern::VariantPatternData::Tuple(patterns) => {
                                // Tuple variant: Some(x, y) - check each positional pattern
                                for (i, pat) in patterns.iter().enumerate() {
                                    let elem_access = SmtExpr::BinOp(
                                        SmtBinOp::Select,
                                        Box::new(inner_access.clone()),
                                        Box::new(SmtExpr::int(i as i64)),
                                    );
                                    let elem_cond = self.pattern_to_condition(pat, &elem_access)?;
                                    if elem_cond != Formula::True {
                                        conditions.push(elem_cond);
                                    }
                                }
                            }
                            verum_ast::pattern::VariantPatternData::Record { fields, .. } => {
                                // Record variant: Error { code, message } - check each field pattern
                                for field in fields.iter() {
                                    let field_access = SmtExpr::Apply(
                                        Text::from(format!("field_{}", field.name.as_str())),
                                        vec![inner_access.clone()].into(),
                                    );
                                    // Handle optional pattern (shorthand notation)
                                    if let Maybe::Some(ref pat) = field.pattern {
                                        let field_cond =
                                            self.pattern_to_condition(pat, &field_access)?;
                                        if field_cond != Formula::True {
                                            conditions.push(field_cond);
                                        }
                                    }
                                }
                            }
                        }

                        Ok(Formula::and(conditions))
                    }
                    Maybe::None => Ok(variant_check),
                }
            }

            // Or pattern: disjunction of alternatives
            PatternKind::Or(patterns) => {
                let mut conditions = List::new();
                for pat in patterns.iter() {
                    conditions.push(self.pattern_to_condition(pat, scrutinee)?);
                }
                Ok(Formula::or(conditions))
            }

            // Range pattern: start <= scrutinee <= end
            PatternKind::Range {
                start,
                end,
                inclusive,
            } => {
                let start_cond = match start {
                    Maybe::Some(s) => {
                        let start_smt = self.literal_to_smt(s)?;
                        Formula::le(start_smt, scrutinee.clone())
                    }
                    Maybe::None => Formula::True,
                };

                let end_cond = match end {
                    Maybe::Some(e) => {
                        let end_smt = self.literal_to_smt(e)?;
                        if *inclusive {
                            Formula::le(scrutinee.clone(), end_smt)
                        } else {
                            Formula::lt(scrutinee.clone(), end_smt)
                        }
                    }
                    Maybe::None => Formula::True,
                };

                Ok(Formula::and(vec![start_cond, end_cond]))
            }

            // Reference pattern: match through the reference
            PatternKind::Reference { inner, .. } => {
                let deref_scrutinee = SmtExpr::UnOp(SmtUnOp::Deref, Box::new(scrutinee.clone()));
                self.pattern_to_condition(inner, &deref_scrutinee)
            }

            // Slice pattern: check length and element patterns
            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                // For slices, we check the before elements and after elements
                let min_len = before.len() + after.len();

                // If there's no rest pattern, the slice must have exact length
                let len_check = if rest.is_none() {
                    Formula::eq(
                        SmtExpr::UnOp(SmtUnOp::Len, Box::new(scrutinee.clone())),
                        SmtExpr::int(min_len as i64),
                    )
                } else {
                    Formula::ge(
                        SmtExpr::UnOp(SmtUnOp::Len, Box::new(scrutinee.clone())),
                        SmtExpr::int(min_len as i64),
                    )
                };

                let mut conditions = vec![len_check];

                // Check 'before' patterns (from the start)
                for (i, pat) in before.iter().enumerate() {
                    let elem_access = SmtExpr::Apply(
                        Text::from("array_get"),
                        vec![scrutinee.clone(), SmtExpr::int(i as i64)].into(),
                    );
                    let elem_cond = self.pattern_to_condition(pat, &elem_access)?;
                    if elem_cond != Formula::True {
                        conditions.push(elem_cond);
                    }
                }

                // Check 'after' patterns (from the end using negative indexing)
                // For [a, b, .., c, d], 'c' is at len-2 and 'd' is at len-1
                let after_len = after.len();
                for (i, pat) in after.iter().enumerate() {
                    // Calculate index from end: len - (after_len - i)
                    // For the last element (i = after_len - 1): len - 1
                    // For second to last (i = after_len - 2): len - 2
                    let offset_from_end = (after_len - 1 - i) as i64;
                    let index_expr = SmtExpr::BinOp(
                        SmtBinOp::Sub,
                        Box::new(SmtExpr::UnOp(SmtUnOp::Len, Box::new(scrutinee.clone()))),
                        Box::new(SmtExpr::int(offset_from_end + 1)),
                    );
                    let elem_access = SmtExpr::Apply(
                        Text::from("array_get"),
                        vec![scrutinee.clone(), index_expr].into(),
                    );
                    let elem_cond = self.pattern_to_condition(pat, &elem_access)?;
                    if elem_cond != Formula::True {
                        conditions.push(elem_cond);
                    }
                }

                // Rest pattern is handled implicitly - it matches anything in between
                let _ = rest;

                Ok(Formula::and(conditions))
            }

            // Rest pattern: always matches remaining elements
            PatternKind::Rest => Ok(Formula::True),

            // Parenthesized pattern: delegate to inner
            PatternKind::Paren(inner) => self.pattern_to_condition(inner, scrutinee),

            // Array pattern
            PatternKind::Array(patterns) => {
                let mut conditions = List::new();
                for (i, pat) in patterns.iter().enumerate() {
                    let elem_access = SmtExpr::Apply(
                        Text::from("array_get"),
                        vec![scrutinee.clone(), SmtExpr::int(i as i64)].into(),
                    );
                    let cond = self.pattern_to_condition(pat, &elem_access)?;
                    conditions.push(cond);
                }
                Ok(Formula::and(conditions))
            }

            // View pattern - always matches (the view function transforms the value)            PatternKind::View { .. } => Ok(Formula::True),

            // Active pattern - requires calling the pattern function
            // For verification purposes, we assume it could match or not
            PatternKind::Active { .. } => Ok(Formula::True),

            // And pattern - all sub-patterns must match
            PatternKind::And(patterns) => {
                let mut conditions = List::new();
                for pat in patterns.iter() {
                    conditions.push(self.pattern_to_condition(pat, scrutinee)?);
                }
                Ok(Formula::and(conditions))
            }

            // Type test pattern - check if scrutinee has the specified type
            PatternKind::TypeTest { test_type, .. } => {
                // Generate type predicate: has_type(scrutinee, TypeName)
                let type_name = format!("{:?}", test_type.kind);
                Ok(Formula::Predicate(
                    Text::from("has_type"),
                    vec![
                        scrutinee.clone(),
                        SmtExpr::var(type_name),
                    ]
                    .into(),
                ))
            }

            // Stream pattern - check length and head element patterns
            PatternKind::Stream { head_patterns, rest } => {
                // Stream patterns consume elements from an iterator
                // For verification, we check:
                // 1. Stream has at least len(head_patterns) elements
                // 2. Each head element matches its pattern
                let min_len = head_patterns.len();

                // If no rest pattern, require exact length
                let len_check = if rest.is_none() {
                    Formula::eq(
                        SmtExpr::UnOp(SmtUnOp::Len, Box::new(scrutinee.clone())),
                        SmtExpr::int(min_len as i64),
                    )
                } else {
                    Formula::ge(
                        SmtExpr::UnOp(SmtUnOp::Len, Box::new(scrutinee.clone())),
                        SmtExpr::int(min_len as i64),
                    )
                };

                let mut conditions = vec![len_check];

                // Check each head pattern against the corresponding element
                for (i, pat) in head_patterns.iter().enumerate() {
                    let elem_access = SmtExpr::Apply(
                        Text::from("stream_get"),
                        vec![scrutinee.clone(), SmtExpr::int(i as i64)].into(),
                    );
                    let elem_cond = self.pattern_to_condition(pat, &elem_access)?;
                    if elem_cond != Formula::True {
                        conditions.push(elem_cond);
                    }
                }

                Ok(Formula::and(conditions))
            }

            PatternKind::Guard { pattern, guard } => {
                // Guard pattern: (pattern if guard)
                // Spec: Rust RFC 3637 - Guard Patterns
                //
                // The pattern matches if both:
                // 1. The inner pattern matches the scrutinee
                // 2. The guard expression evaluates to true
                let inner_cond = self.pattern_to_condition(pattern, scrutinee)?;
                let guard_cond = self.expr_to_formula(guard)?;
                Ok(Formula::and(vec![inner_cond, guard_cond]))
            }

            #[allow(deprecated)]
            PatternKind::View { pattern, .. } => {
                // View patterns: match the inner pattern
                self.pattern_to_condition(pattern, scrutinee)
            }
            PatternKind::Cons { .. } => {
                // Cons pattern: head :: tail — treat as constructor match
                Ok(Formula::True)
            }
        }
    }

    /// Extract variable bindings from a pattern
    ///
    /// Returns a list of (variable_name, expression_to_extract) pairs
    /// that represent the bindings introduced by the pattern.
    fn extract_pattern_bindings(
        &self,
        pattern: &Pattern,
        scrutinee: &SmtExpr,
    ) -> Result<List<(Text, SmtExpr)>, WPError> {
        let mut bindings = List::new();
        self.extract_bindings_recursive(pattern, scrutinee, &mut bindings)?;
        Ok(bindings)
    }

    fn extract_bindings_recursive(
        &self,
        pattern: &Pattern,
        scrutinee: &SmtExpr,
        bindings: &mut List<(Text, SmtExpr)>,
    ) -> Result<(), WPError> {
        match &pattern.kind {
            PatternKind::Wildcard => Ok(()),

            PatternKind::Ident {
                name, subpattern, ..
            } => {
                // Bind the name and also extract bindings from sub-pattern
                bindings.push((Text::from(name.as_str()), scrutinee.clone()));
                if let Maybe::Some(sub_pat) = subpattern {
                    self.extract_bindings_recursive(sub_pat, scrutinee, bindings)?;
                }
                Ok(())
            }

            PatternKind::Tuple(patterns) => {
                for (i, pat) in patterns.iter().enumerate() {
                    let elem_access = SmtExpr::BinOp(
                        SmtBinOp::Select,
                        Box::new(scrutinee.clone()),
                        Box::new(SmtExpr::int(i as i64)),
                    );
                    self.extract_bindings_recursive(pat, &elem_access, bindings)?;
                }
                Ok(())
            }

            PatternKind::Record { fields, .. } => {
                for field in fields.iter() {
                    let field_access = SmtExpr::BinOp(
                        SmtBinOp::Select,
                        Box::new(scrutinee.clone()),
                        Box::new(SmtExpr::var(format!("field_{}", field.name.as_str()))),
                    );
                    // Field pattern may be None for shorthand form { x }
                    if let Maybe::Some(ref pat) = field.pattern {
                        self.extract_bindings_recursive(pat, &field_access, bindings)?;
                    } else {
                        // Shorthand: the field name itself is bound
                        bindings.push((Text::from(field.name.as_str()), field_access));
                    }
                }
                Ok(())
            }

            PatternKind::Variant { data, .. } => {
                if let Maybe::Some(variant_data) = data {
                    let inner_access =
                        SmtExpr::UnOp(SmtUnOp::GetVariantValue, Box::new(scrutinee.clone()));
                    // Handle both tuple and record variant patterns
                    match variant_data {
                        verum_ast::pattern::VariantPatternData::Tuple(patterns) => {
                            for (i, pat) in patterns.iter().enumerate() {
                                let elem_access = SmtExpr::BinOp(
                                    SmtBinOp::Select,
                                    Box::new(inner_access.clone()),
                                    Box::new(SmtExpr::int(i as i64)),
                                );
                                self.extract_bindings_recursive(pat, &elem_access, bindings)?;
                            }
                        }
                        verum_ast::pattern::VariantPatternData::Record { fields, .. } => {
                            for field in fields.iter() {
                                let field_access = SmtExpr::BinOp(
                                    SmtBinOp::Select,
                                    Box::new(inner_access.clone()),
                                    Box::new(SmtExpr::var(format!(
                                        "field_{}",
                                        field.name.as_str()
                                    ))),
                                );
                                if let Maybe::Some(ref pat) = field.pattern {
                                    self.extract_bindings_recursive(pat, &field_access, bindings)?;
                                } else {
                                    bindings.push((Text::from(field.name.as_str()), field_access));
                                }
                            }
                        }
                    }
                }
                Ok(())
            }

            PatternKind::Reference { inner, .. } => {
                let deref = SmtExpr::UnOp(SmtUnOp::Deref, Box::new(scrutinee.clone()));
                self.extract_bindings_recursive(inner, &deref, bindings)
            }

            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                // Extract bindings from 'before' patterns (from the start)
                for (i, pat) in before.iter().enumerate() {
                    let elem_access = SmtExpr::BinOp(
                        SmtBinOp::Select,
                        Box::new(scrutinee.clone()),
                        Box::new(SmtExpr::int(i as i64)),
                    );
                    self.extract_bindings_recursive(pat, &elem_access, bindings)?;
                }

                // Handle rest pattern if present
                // The rest binds to a sub-slice from index before.len() to len - after.len()
                if let Some(rest_pat) = rest {
                    let before_len = before.len() as i64;
                    let after_len = after.len() as i64;
                    // slice_range(arr, start, end) extracts a sub-slice
                    let rest_access = SmtExpr::Apply(
                        Text::from("slice_range"),
                        vec![
                            scrutinee.clone(),
                            SmtExpr::int(before_len),
                            SmtExpr::BinOp(
                                SmtBinOp::Sub,
                                Box::new(SmtExpr::UnOp(SmtUnOp::Len, Box::new(scrutinee.clone()))),
                                Box::new(SmtExpr::int(after_len)),
                            ),
                        ]
                        .into(),
                    );
                    self.extract_bindings_recursive(rest_pat, &rest_access, bindings)?;
                }

                // Extract bindings from 'after' patterns (from the end)
                // For [a, b, .., c, d], 'c' is at len-2 and 'd' is at len-1
                let after_len = after.len();
                for (i, pat) in after.iter().enumerate() {
                    let offset_from_end = (after_len - 1 - i) as i64;
                    let index_expr = SmtExpr::BinOp(
                        SmtBinOp::Sub,
                        Box::new(SmtExpr::UnOp(SmtUnOp::Len, Box::new(scrutinee.clone()))),
                        Box::new(SmtExpr::int(offset_from_end + 1)),
                    );
                    let elem_access = SmtExpr::BinOp(
                        SmtBinOp::Select,
                        Box::new(scrutinee.clone()),
                        Box::new(index_expr),
                    );
                    self.extract_bindings_recursive(pat, &elem_access, bindings)?;
                }
                Ok(())
            }

            PatternKind::Or(patterns) => {
                // For or patterns, extract bindings from first alternative
                // (all alternatives must bind the same names)
                if let Some(first) = patterns.first() {
                    self.extract_bindings_recursive(first, scrutinee, bindings)?;
                }
                Ok(())
            }

            PatternKind::Paren(inner) => {
                self.extract_bindings_recursive(inner, scrutinee, bindings)
            }

            _ => Ok(()), // Other patterns don't introduce bindings
        }
    }

    /// Convert a pattern literal to SMT expression
    fn pattern_to_smt_expr(&self, pattern: &Pattern) -> Result<SmtExpr, WPError> {
        match &pattern.kind {
            PatternKind::Literal(lit) => self.literal_to_smt(lit),
            _ => Ok(SmtExpr::var("pattern")),
        }
    }

    /// Convert a literal expression to SMT
    fn literal_to_smt(&self, lit: &verum_ast::Literal) -> Result<SmtExpr, WPError> {
        match &lit.kind {
            verum_ast::LiteralKind::Int(n) => Ok(SmtExpr::int(n.value as i64)),
            verum_ast::LiteralKind::Bool(b) => Ok(SmtExpr::bool(*b)),
            verum_ast::LiteralKind::Char(c) => Ok(SmtExpr::int(*c as i64)),
            verum_ast::LiteralKind::Float(f) => Ok(SmtExpr::real(f.value)),
            verum_ast::LiteralKind::Text(s) => Ok(SmtExpr::var(format!("str_{}", s.as_str()))),
            _ => Ok(SmtExpr::var("literal")),
        }
    }

    /// Extract function name from a function expression
    fn expr_to_func_name(&self, expr: &Expr) -> Result<Text, WPError> {
        match &expr.kind {
            ExprKind::Path(path) => Ok(path
                .segments
                .iter()
                .map(|s| path_segment_to_str(s).to_string())
                .collect::<Vec<_>>()
                .join(".")
                .into()),
            _ => Ok(Text::from("unknown_func")),
        }
    }

    /// Get statistics
    pub fn stats(&self) -> &HoareStats {
        &self.stats
    }

    /// Add symbol to the symbol table
    pub fn add_symbol(&mut self, name: Text, ty: VarType) {
        self.wp_calculator.add_symbol(name, ty);
    }
}

impl Default for HoareLogic {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Frame Rule Support
// =============================================================================

/// Frame rule for local reasoning
///
/// The frame rule allows reasoning about local state changes:
/// {P} c {Q}
/// ─────────────────────────────  (frame: vars(R) ∩ mod(c) = ∅)
/// {P ∧ R} c {Q ∧ R}
///
/// If c doesn't modify variables in R, then R is preserved.
#[derive(Debug)]
pub struct FrameRule;

impl FrameRule {
    /// Apply frame rule to extend a triple with frame condition
    pub fn apply(
        triple: &HoareTriple,
        frame: &Formula,
        wp_calc: &WPCalculator,
    ) -> Result<HoareTriple, WPError> {
        // Check that frame variables are not modified by command
        let modified = wp_calc.collect_modified_vars(&triple.command);
        let frame_vars = frame.free_variables();

        for var in &frame_vars {
            if modified.contains(var) {
                return Err(WPError::FrameViolation {
                    var: var.clone(),
                    frame: frame.to_smtlib(),
                });
            }
        }

        // Build framed triple: {P ∧ R} c {Q ∧ R}
        Ok(HoareTriple {
            precondition: Formula::and(vec![triple.precondition.clone(), frame.clone()]),
            command: triple.command.clone(),
            postcondition: Formula::and(vec![triple.postcondition.clone(), frame.clone()]),
            location: triple.location.clone(),
        })
    }

    /// Check if a variable set is disjoint from modified variables
    pub fn is_frame_safe(
        command: &Command,
        frame_vars: &HashSet<Variable>,
        wp_calc: &WPCalculator,
    ) -> bool {
        let modified = wp_calc.collect_modified_vars(command);
        frame_vars.is_disjoint(&modified)
    }
}

// =============================================================================
// Supporting Types
// =============================================================================

/// Verification condition with metadata
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationCondition {
    /// The formula to verify
    pub formula: Formula,
    /// Original Hoare triple
    pub triple: HoareTriple,
    /// Kind of VC
    pub kind: VCKind,
    /// Source location
    pub location: Maybe<SourceLocation>,
}

impl VerificationCondition {
    /// Convert to SMT-LIB format
    pub fn to_smtlib(&self) -> Text {
        self.formula.to_smtlib()
    }

    /// Get a human-readable description
    pub fn description(&self) -> Text {
        match &self.kind {
            VCKind::HoareTriple => Text::from("Hoare triple correctness"),
            VCKind::LoopInvariant => Text::from("Loop invariant preservation"),
            VCKind::LoopTermination => Text::from("Loop termination"),
            VCKind::AssertionCheck => Text::from("Assertion validity"),
            VCKind::FrameCondition => Text::from("Frame condition preservation"),
        }
    }
}

/// Kind of verification condition
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VCKind {
    /// Main Hoare triple VC
    HoareTriple,
    /// Loop invariant preservation
    LoopInvariant,
    /// Loop termination
    LoopTermination,
    /// Assertion check
    AssertionCheck,
    /// Frame condition
    FrameCondition,
}

/// Function contract for VC generation
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionContract {
    /// Function name
    pub name: Text,
    /// Precondition (requires clause)
    pub requires: Formula,
    /// Postcondition (ensures clause)
    pub ensures: Formula,
    /// Function body
    pub body: FunctionBody,
    /// Modifies clause (frame condition)
    pub modifies: HashSet<Variable>,
}

/// Function body representation
#[derive(Debug, Clone, PartialEq)]
pub enum FunctionBody {
    /// Expression body
    Expr(Expr),
    /// Statement list
    Block(List<Stmt>),
}

/// Statistics for Hoare logic verification
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HoareStats {
    /// Number of VCs generated
    pub vc_count: usize,
    /// Number of successful verifications (VC is valid)
    pub verified_count: usize,
    /// Number of failed verifications (VC is invalid, counterexample found)
    pub failed_count: usize,
    /// Number of unknown results (solver timeout or resource limit)
    pub unknown_count: usize,
    /// Total verification time (ms)
    pub total_time_ms: u64,
}

impl HoareStats {
    /// Get success rate
    pub fn success_rate(&self) -> f64 {
        let total = self.verified_count + self.failed_count;
        if total == 0 {
            0.0
        } else {
            self.verified_count as f64 / total as f64
        }
    }

    /// Get average verification time
    pub fn avg_time_ms(&self) -> f64 {
        let total = self.verified_count + self.failed_count;
        if total == 0 {
            0.0
        } else {
            self.total_time_ms as f64 / total as f64
        }
    }
}

// =============================================================================
// Error Types
// =============================================================================

/// Errors that can occur during WP calculation or verification
#[derive(Debug, Clone, thiserror::Error)]
pub enum WPError {
    /// Invalid command structure
    #[error("invalid command: {0}")]
    InvalidCommand(Text),

    /// Missing loop invariant
    #[error("missing loop invariant at {location:?}")]
    MissingInvariant { location: Maybe<SourceLocation> },

    /// Frame rule violation
    #[error("frame rule violation: variable {var} is modified but appears in frame {frame}")]
    FrameViolation { var: Variable, frame: Text },

    /// Type error in formula
    #[error("type error in formula: {0}")]
    TypeError(Text),

    /// Unsupported construct
    #[error("unsupported construct: {0}")]
    Unsupported(Text),

    /// Conversion error
    #[error("conversion error: {0}")]
    ConversionError(Text),

    /// SMT solver error
    #[error("SMT solver error: {0}")]
    SmtError(Text),

    /// Verification failed with counterexample
    #[error("verification failed: {message}")]
    VerificationFailed {
        /// Human-readable failure message
        message: Text,
        /// Counterexample demonstrating the failure
        counterexample: Maybe<CounterExample>,
        /// Source location where verification failed
        location: Maybe<SourceLocation>,
    },

    /// Solver returned unknown (timeout or resource limit)
    #[error("solver returned unknown: {reason}")]
    Unknown {
        /// Reason for unknown result
        reason: Text,
        /// Source location
        location: Maybe<SourceLocation>,
    },
}

// =============================================================================
// Public API Functions
// =============================================================================

/// Compute weakest precondition for a command and postcondition
///
/// This is a convenience function that creates a WPCalculator and computes WP.
pub fn wp(command: &Command, postcondition: &Formula) -> Result<Formula, WPError> {
    let calculator = WPCalculator::new();
    calculator.wp(command, postcondition)
}

/// Generate verification condition from a Hoare triple
///
/// This is a convenience function that creates a HoareLogic instance and generates VC.
pub fn generate_vc(triple: &HoareTriple) -> Result<VerificationCondition, WPError> {
    let logic = HoareLogic::new();
    logic.generate_vc(triple)
}

/// Apply frame rule to a Hoare triple
pub fn apply_frame(triple: &HoareTriple, frame: &Formula) -> Result<HoareTriple, WPError> {
    let wp_calc = WPCalculator::new();
    FrameRule::apply(triple, frame, &wp_calc)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wp_skip() {
        let calc = WPCalculator::new();
        let q = Formula::eq(SmtExpr::var("x"), SmtExpr::int(5));
        let wp_result = calc.wp(&Command::Skip, &q).unwrap();
        assert_eq!(wp_result, q);
    }

    #[test]
    fn test_wp_assign() {
        let calc = WPCalculator::new();
        // wp(x := x + 1, x > 0) = x + 1 > 0
        let q = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));
        let cmd = Command::Assign {
            var: Variable::new("x"),
            expr: SmtExpr::add(SmtExpr::var("x"), SmtExpr::int(1)),
        };
        let wp_result = calc.wp(&cmd, &q).unwrap();
        // Should be: x + 1 > 0
        assert_eq!(
            wp_result,
            Formula::gt(
                SmtExpr::add(SmtExpr::var("x"), SmtExpr::int(1)),
                SmtExpr::int(0)
            )
        );
    }

    #[test]
    fn test_wp_seq() {
        let calc = WPCalculator::new();
        // wp(x := 1; y := x, y = 1) = wp(x := 1, wp(y := x, y = 1))
        //                            = wp(x := 1, x = 1)
        //                            = 1 = 1 = true
        let q = Formula::eq(SmtExpr::var("y"), SmtExpr::int(1));

        let cmd1 = Command::Assign {
            var: Variable::new("x"),
            expr: SmtExpr::int(1),
        };
        let cmd2 = Command::Assign {
            var: Variable::new("y"),
            expr: SmtExpr::var("x"),
        };
        let seq = Command::Seq {
            first: Heap::new(cmd1),
            second: Heap::new(cmd2),
        };

        let wp_result = calc.wp(&seq, &q).unwrap();
        // Should be: 1 = 1 which is True
        assert_eq!(wp_result, Formula::eq(SmtExpr::int(1), SmtExpr::int(1)));
    }

    #[test]
    fn test_wp_if() {
        let calc = WPCalculator::new();
        // wp(if x > 0 then y := 1 else y := 2, y > 0)
        //   = (x > 0 => y := 1 > 0) ∧ (x <= 0 => y := 2 > 0)
        //   = (x > 0 => 1 > 0) ∧ (x <= 0 => 2 > 0)
        //   = true (since both branches satisfy y > 0)

        let q = Formula::gt(SmtExpr::var("y"), SmtExpr::int(0));
        let cond = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));

        let then_cmd = Command::Assign {
            var: Variable::new("y"),
            expr: SmtExpr::int(1),
        };
        let else_cmd = Command::Assign {
            var: Variable::new("y"),
            expr: SmtExpr::int(2),
        };

        let if_cmd = Command::If {
            condition: cond.clone(),
            then_branch: Heap::new(then_cmd),
            else_branch: Maybe::Some(Heap::new(else_cmd)),
        };

        let wp_result = calc.wp(&if_cmd, &q).unwrap();

        // Result should be conjunction of implications
        match wp_result {
            Formula::And(fs) => assert_eq!(fs.len(), 2),
            _ => panic!("Expected And formula"),
        }
    }

    #[test]
    fn test_hoare_triple_creation() {
        let pre = Formula::ge(SmtExpr::var("x"), SmtExpr::int(0));
        let post = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));
        let cmd = Command::Assign {
            var: Variable::new("x"),
            expr: SmtExpr::add(SmtExpr::var("x"), SmtExpr::int(1)),
        };

        let triple = HoareTriple::new(pre, cmd, post);
        let formatted = triple.format();
        assert!(formatted.contains(">="));
        assert!(formatted.contains(":="));
    }

    #[test]
    fn test_vc_generation() {
        let logic = HoareLogic::new();
        let pre = Formula::ge(SmtExpr::var("x"), SmtExpr::int(0));
        let post = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));
        let cmd = Command::Assign {
            var: Variable::new("x"),
            expr: SmtExpr::add(SmtExpr::var("x"), SmtExpr::int(1)),
        };

        let triple = HoareTriple::new(pre, cmd, post);
        let vc = logic.generate_vc(&triple).unwrap();

        // VC should be: (x >= 0) => (x + 1 > 0)
        match &vc.formula {
            Formula::Implies(_, _) => {}
            _ => panic!("Expected Implies formula"),
        }
    }

    #[test]
    fn test_frame_rule() {
        let wp_calc = WPCalculator::new();
        let pre = Formula::True;
        let post = Formula::eq(SmtExpr::var("x"), SmtExpr::int(1));
        let cmd = Command::Assign {
            var: Variable::new("x"),
            expr: SmtExpr::int(1),
        };

        let triple = HoareTriple::new(pre, cmd, post);
        let frame = Formula::eq(SmtExpr::var("y"), SmtExpr::int(5));

        // This should succeed because y is not modified
        let framed = FrameRule::apply(&triple, &frame, &wp_calc).unwrap();

        // Postcondition should be: x = 1 ∧ y = 5
        match &framed.postcondition {
            Formula::And(fs) => assert_eq!(fs.len(), 2),
            _ => panic!("Expected And formula"),
        }
    }

    #[test]
    fn test_frame_violation() {
        let wp_calc = WPCalculator::new();
        let pre = Formula::True;
        let post = Formula::eq(SmtExpr::var("x"), SmtExpr::int(1));
        let cmd = Command::Assign {
            var: Variable::new("x"),
            expr: SmtExpr::int(1),
        };

        let triple = HoareTriple::new(pre, cmd, post);
        let frame = Formula::eq(SmtExpr::var("x"), SmtExpr::int(0)); // x is modified!

        // This should fail because x is modified
        let result = FrameRule::apply(&triple, &frame, &wp_calc);
        assert!(result.is_err());
        match result {
            Err(WPError::FrameViolation { .. }) => {}
            _ => panic!("Expected FrameViolation error"),
        }
    }

    #[test]
    fn test_assert_command() {
        let calc = WPCalculator::new();
        let p = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));
        let q = Formula::gt(SmtExpr::var("y"), SmtExpr::int(0));
        let cmd = Command::Assert(p.clone());

        // wp(assert(P), Q) = P ∧ Q
        let wp_result = calc.wp(&cmd, &q).unwrap();
        match wp_result {
            Formula::And(fs) => assert_eq!(fs.len(), 2),
            _ => panic!("Expected And formula"),
        }
    }

    #[test]
    fn test_array_update() {
        let calc = WPCalculator::new();
        // wp(arr[i] := v, arr[i] = v) should be: store(arr, i, v)[i] = v
        let arr_var = Variable::new("arr");
        let idx = SmtExpr::var("i");
        let val = SmtExpr::var("v");

        let cmd = Command::ArrayUpdate {
            array: arr_var.clone(),
            index: idx.clone(),
            value: val.clone(),
        };

        let post = Formula::eq(
            SmtExpr::Select(Box::new(SmtExpr::Var(arr_var)), Box::new(idx)),
            val,
        );

        let wp_result = calc.wp(&cmd, &post).unwrap();
        // Should involve store operation
        assert!(format!("{:?}", wp_result).contains("Store"));
    }

    // =========================================================================
    // Termination Verification Tests
    // =========================================================================

    #[test]
    fn test_wp_while_with_single_measure_termination() {
        let calc = WPCalculator::new();
        // While loop: while n > 0 inv n >= 0 decreases n { n := n - 1 }
        let cond = Formula::gt(SmtExpr::var("n"), SmtExpr::int(0));
        let inv = Formula::ge(SmtExpr::var("n"), SmtExpr::int(0));
        let body = Command::Assign {
            var: Variable::new("n"),
            expr: SmtExpr::sub(SmtExpr::var("n"), SmtExpr::int(1)),
        };
        let measure = SmtExpr::var("n");

        let while_cmd = Command::While {
            condition: cond,
            invariant: inv,
            body: Heap::new(body),
            decreases: Maybe::Some(measure),
            lexicographic_decreases: Maybe::None,
        };

        let post = Formula::eq(SmtExpr::var("n"), SmtExpr::int(0));
        let wp_result = calc.wp(&while_cmd, &post).unwrap();

        // The result should contain termination VCs
        let debug_str = format!("{:?}", wp_result);
        // Should contain invariant, preservation, exit, and termination conditions
        assert!(debug_str.contains("And"));
    }

    #[test]
    fn test_wp_while_with_lexicographic_termination() {
        let calc = WPCalculator::new();
        // While loop with lexicographic measure (i, j)
        // while i > 0 || j > 0 inv true decreases (i, j) { ... }
        let cond = Formula::or(vec![
            Formula::gt(SmtExpr::var("i"), SmtExpr::int(0)),
            Formula::gt(SmtExpr::var("j"), SmtExpr::int(0)),
        ]);
        let inv = Formula::True;

        // Body: if j > 0 then j := j - 1 else { i := i - 1; j := 10 }
        let body = Command::If {
            condition: Formula::gt(SmtExpr::var("j"), SmtExpr::int(0)),
            then_branch: Heap::new(Command::Assign {
                var: Variable::new("j"),
                expr: SmtExpr::sub(SmtExpr::var("j"), SmtExpr::int(1)),
            }),
            else_branch: Maybe::Some(Heap::new(Command::Seq {
                first: Heap::new(Command::Assign {
                    var: Variable::new("i"),
                    expr: SmtExpr::sub(SmtExpr::var("i"), SmtExpr::int(1)),
                }),
                second: Heap::new(Command::Assign {
                    var: Variable::new("j"),
                    expr: SmtExpr::int(10),
                }),
            })),
        };

        let measures: List<SmtExpr> = vec![SmtExpr::var("i"), SmtExpr::var("j")].into();

        let while_cmd = Command::While {
            condition: cond,
            invariant: inv,
            body: Heap::new(body),
            decreases: Maybe::None,
            lexicographic_decreases: Maybe::Some(measures),
        };

        let post = Formula::True;
        let wp_result = calc.wp(&while_cmd, &post).unwrap();

        // The result should contain lexicographic termination VCs
        let debug_str = format!("{:?}", wp_result);
        // Should have well-foundedness for both measures and lexicographic decrease
        assert!(debug_str.contains("And"));
    }

    #[test]
    fn test_single_measure_termination_vc_generation() {
        let calc = WPCalculator::new();

        let invariant = Formula::ge(SmtExpr::var("x"), SmtExpr::int(0));
        let condition = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));
        let body = Command::Assign {
            var: Variable::new("x"),
            expr: SmtExpr::sub(SmtExpr::var("x"), SmtExpr::int(1)),
        };
        let measure = SmtExpr::var("x");

        let vcs = calc
            .generate_single_measure_termination_vcs(&invariant, &condition, &body, &measure)
            .unwrap();

        // Should generate exactly 2 VCs:
        // 1. Well-foundedness: I ∧ b => measure >= 0
        // 2. Strict decrease: ∀m_before. I ∧ b ∧ m = m_before => wp(body, m < m_before)
        assert_eq!(vcs.len(), 2);

        // First VC should be the well-foundedness condition
        match &vcs[0] {
            Formula::Implies(_, consequent) => {
                // Consequent should be measure >= 0
                assert!(matches!(consequent.as_ref(), Formula::Ge(_, _)));
            }
            _ => panic!("Expected Implies formula for well-foundedness"),
        }

        // Second VC should be quantified termination condition
        match &vcs[1] {
            Formula::Forall(vars, _) => {
                assert_eq!(vars.len(), 1);
                assert!(vars[0].name.starts_with("__measure_before"));
            }
            _ => panic!("Expected Forall formula for termination"),
        }
    }

    #[test]
    fn test_lexicographic_termination_vc_generation() {
        let calc = WPCalculator::new();

        let invariant = Formula::and(vec![
            Formula::ge(SmtExpr::var("x"), SmtExpr::int(0)),
            Formula::ge(SmtExpr::var("y"), SmtExpr::int(0)),
        ]);
        let condition = Formula::or(vec![
            Formula::gt(SmtExpr::var("x"), SmtExpr::int(0)),
            Formula::gt(SmtExpr::var("y"), SmtExpr::int(0)),
        ]);
        let body = Command::Assign {
            var: Variable::new("y"),
            expr: SmtExpr::sub(SmtExpr::var("y"), SmtExpr::int(1)),
        };
        let measures: List<SmtExpr> = vec![SmtExpr::var("x"), SmtExpr::var("y")].into();

        let vcs = calc
            .generate_lexicographic_termination_vcs(&invariant, &condition, &body, &measures)
            .unwrap();

        // Should generate exactly 2 VCs:
        // 1. Well-foundedness: I ∧ b => (m1 >= 0 ∧ m2 >= 0)
        // 2. Lexicographic decrease: ∀m1_before, m2_before. ... => wp(body, lex_lt)
        assert_eq!(vcs.len(), 2);

        // First VC should be well-foundedness for both measures
        match &vcs[0] {
            Formula::Implies(_, consequent) => {
                // Consequent should be conjunction of >= 0 conditions
                assert!(matches!(consequent.as_ref(), Formula::And(_)));
            }
            _ => panic!("Expected Implies formula for well-foundedness"),
        }

        // Second VC should be quantified over all measure snapshot variables
        match &vcs[1] {
            Formula::Forall(vars, _) => {
                // Should have 2 quantified variables (one per measure)
                assert_eq!(vars.len(), 2);
            }
            _ => panic!("Expected Forall formula for lexicographic termination"),
        }
    }

    #[test]
    fn test_build_lexicographic_decrease() {
        let calc = WPCalculator::new();

        // Test with 2 measures
        let current: List<SmtExpr> = vec![SmtExpr::var("x"), SmtExpr::var("y")].into();
        let before = vec![SmtExpr::var("x0"), SmtExpr::var("y0")];

        let formula = calc.build_lexicographic_decrease(&current, &before);

        // Should be: (x < x0) OR (x = x0 AND y < y0)
        match formula {
            Formula::Or(disjuncts) => {
                assert_eq!(disjuncts.len(), 2);

                // First disjunct: x < x0
                assert!(matches!(&disjuncts[0], Formula::Lt(_, _)));

                // Second disjunct: x = x0 AND y < y0
                match &disjuncts[1] {
                    Formula::And(conjuncts) => {
                        assert_eq!(conjuncts.len(), 2);
                        assert!(matches!(&conjuncts[0], Formula::Eq(_, _)));
                        assert!(matches!(&conjuncts[1], Formula::Lt(_, _)));
                    }
                    _ => panic!("Expected And formula"),
                }
            }
            _ => panic!("Expected Or formula for lexicographic decrease"),
        }
    }

    #[test]
    fn test_build_lexicographic_decrease_three_measures() {
        let calc = WPCalculator::new();

        // Test with 3 measures
        let current: List<SmtExpr> =
            vec![SmtExpr::var("x"), SmtExpr::var("y"), SmtExpr::var("z")].into();
        let before = vec![SmtExpr::var("x0"), SmtExpr::var("y0"), SmtExpr::var("z0")];

        let formula = calc.build_lexicographic_decrease(&current, &before);

        // Should be:
        //   (x < x0) OR
        //   (x = x0 AND y < y0) OR
        //   (x = x0 AND y = y0 AND z < z0)
        match formula {
            Formula::Or(disjuncts) => {
                assert_eq!(disjuncts.len(), 3);
            }
            _ => panic!("Expected Or formula with 3 disjuncts"),
        }
    }

    #[test]
    fn test_build_lexicographic_decrease_single_measure() {
        let calc = WPCalculator::new();

        // Single measure should just be strict decrease
        let current: List<SmtExpr> = vec![SmtExpr::var("x")].into();
        let before = vec![SmtExpr::var("x0")];

        let formula = calc.build_lexicographic_decrease(&current, &before);

        // Should be: x < x0 (wrapped in Or with one element, which simplifies)
        // After Formula::or simplification, single element becomes the element itself
        assert!(matches!(formula, Formula::Lt(_, _)));
    }

    #[test]
    fn test_build_lexicographic_decrease_empty() {
        let calc = WPCalculator::new();

        // Empty measures should return False
        let current: List<SmtExpr> = vec![].into();
        let before: Vec<SmtExpr> = vec![];

        let formula = calc.build_lexicographic_decrease(&current, &before);

        assert!(matches!(formula, Formula::False));
    }

    #[test]
    fn test_for_loop_termination() {
        let calc = WPCalculator::new();
        // For loop: for i in 0..n { ... }
        // Should automatically generate decreases clause: n - i
        let body = Command::Skip;

        let for_cmd = Command::For {
            var: Variable::new("i"),
            start: SmtExpr::int(0),
            end: SmtExpr::var("n"),
            invariant: Formula::True,
            body: Heap::new(body),
        };

        let post = Formula::True;
        let wp_result = calc.wp(&for_cmd, &post).unwrap();

        // For loop desugars to while loop with automatic termination measure
        // Should contain termination VCs
        let debug_str = format!("{:?}", wp_result);
        assert!(debug_str.contains("And"));
    }

    #[test]
    fn test_validate_measure() {
        let calc = WPCalculator::new();

        // Valid measures
        assert!(calc.validate_measure(&SmtExpr::int(5)));
        assert!(calc.validate_measure(&SmtExpr::var("x")));
        assert!(calc.validate_measure(&SmtExpr::sub(SmtExpr::var("n"), SmtExpr::var("i"))));
        assert!(calc.validate_measure(&SmtExpr::UnOp(SmtUnOp::Len, Box::new(SmtExpr::var("arr")))));
        assert!(calc.validate_measure(&SmtExpr::UnOp(SmtUnOp::Abs, Box::new(SmtExpr::var("x")))));
    }
}
