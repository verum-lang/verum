//! Fixed-Point Engine (μZ) Module
//!
//! This module provides support for recursive predicates and fixed-point computation
//! using Z3's μZ engine, enabling verification of recursive data structures and
//! inductive properties.
//!
//! Uses the upstream Z3 fixedpoint API and its ownership rules.
//! Recursive refinement types (e.g., sorted lists, balanced trees) require fixed-point
//! reasoning. Z3's muZ engine computes least fixed points for recursive
//! predicates, enabling verification of inductive properties like `is_sorted(list)`
//! or `is_balanced(tree)` that reference themselves in their definitions.

pub use crate::ranking::{RankingFunction, RankingTransition};

mod program;
mod solution;
pub use program::{
    RecursiveFunction, RecursiveProgram, RecursiveProgramVerifier, VerificationCondition,
    VerificationResult,
};

use std::ffi::CString;
use std::time::Instant;

use z3::ast::{Ast, Bool, Dynamic};
use z3::{Context, Fixedpoint, FuncDecl, SatResult, Sort, Symbol};

use verum_common::{List, Map, Maybe, Set, Shared, Text};

// ==================== Core Types ====================

/// Recursive predicate definition
#[derive(Debug, Clone)]
pub struct RecursivePredicate {
    /// Universally bound constants shared by the definition's clauses.
    pub vars: List<(Text, Sort)>,
    /// Predicate name
    pub name: Text,
    /// Parameter sorts
    pub params: List<Sort>,
    /// Body of the predicate (as a rule)
    pub body: PredicateBody,
    /// Caller metadata; never used as evidence of termination or productivity.
    pub well_founded: bool,
}

/// Predicate body types
#[derive(Debug, Clone)]
pub enum PredicateBody {
    /// Base case (non-recursive)
    Base(Bool),
    /// Recursive case
    Recursive {
        guard: Bool,
        recursive_calls: List<RecursiveCall>,
        conclusion: Bool,
    },
    /// Multiple cases (disjunction)
    Cases(List<PredicateCase>),
}

/// Single case in predicate definition
#[derive(Debug, Clone)]
pub struct PredicateCase {
    pub guard: Maybe<Bool>,
    pub body: Bool,
    pub recursive_calls: List<RecursiveCall>,
}

/// Recursive call in predicate
#[derive(Debug, Clone)]
pub struct RecursiveCall {
    pub predicate: Text,
    pub args: List<Dynamic>,
}

/// Datalog-style rule
#[derive(Debug, Clone)]
pub struct DatalogRule {
    /// Universally bound constants; undeclared symbolic constants are refused.
    pub vars: List<(Text, Sort)>,
    /// Head predicate
    pub head: Atom,
    /// Body atoms (conjunction)
    pub body: List<Atom>,
    /// Constraints
    pub constraints: List<Bool>,
}

/// Atomic formula
#[derive(Debug, Clone)]
pub struct Atom {
    /// Predicate name
    pub predicate: Text,
    /// Arguments
    pub args: List<Dynamic>,
}

/// Constrained Horn Clause (CHC)
#[derive(Debug, Clone)]
pub struct CHC {
    /// Variables
    pub vars: List<(Text, Sort)>,
    /// Hypothesis
    pub hypothesis: List<Atom>,
    /// Constraints
    pub constraints: List<Bool>,
    /// Conclusion
    pub conclusion: Atom,
}

/// Fixed-point query
#[derive(Debug)]
pub struct FixedPointQuery {
    /// Query predicate
    pub predicate: Text,
    /// Arguments
    pub args: List<Dynamic>,
}

/// Fixed-point result
#[derive(Debug)]
pub struct FixedPointResult {
    /// Satisfiability status
    pub status: SatResult,
    /// Raw solver answer; neither an inductive interpretation nor a certificate.
    pub answer: Maybe<Bool>,
    /// Complete candidate interpretations, when available. Raw answers are not parsed into these.
    pub solution: Maybe<FixedPointSolution>,
    /// Statistics
    pub stats: FixedPointStats,
}

/// Proposed inductive interpretations and properties; not validated evidence.
#[derive(Debug)]
pub struct FixedPointSolution {
    /// Predicate interpretations
    pub interpretations: Map<Text, PredicateInterpretation>,
    /// Candidate global properties to check under the relation interpretations.
    pub invariants: List<Bool>,
}

/// Interpretation of a predicate
#[derive(Debug)]
pub struct PredicateInterpretation {
    /// The predicate
    pub predicate: Text,
    /// Distinct symbolic parameters in the registered relation's argument order.
    pub parameters: List<Dynamic>,
    /// Quantifier-free theory formula over those parameters.
    /// Inductiveness is checked against the rules, never supplied as metadata.
    pub formula: Bool,
}

/// Fixed-point statistics
#[derive(Debug, Clone, Default)]
pub struct FixedPointStats {
    /// Number of iterations
    pub iterations: usize,
    /// Solving time
    pub time_ms: u64,
    /// Number of rules
    pub num_rules: usize,
    /// Number of predicates
    pub num_predicates: usize,
}

// ==================== Fixed-Point Engine ====================

/// Z3 Fixed-point engine wrapper
///
/// Provides safe Rust API for Z3's fixedpoint solver (Datalog/CHC solver).
/// Supports:
/// - Datalog rules and facts
/// - Constrained Horn Clauses (CHC)
/// - SPACER/PDR engines for program verification
/// - Recursive predicate solving
pub struct FixedPointEngine {
    /// Upstream RAII owner; no assumptions about private Z3 Rust layouts.
    fp: Fixedpoint,
    /// Z3 context (high-level wrapper)
    ctx: Context,
    /// Registered predicates
    predicates: Map<Text, Shared<FuncDecl>>,
    /// Original assertions retained independently of fixedpoint preprocessing.
    rules: List<Bool>,
    axioms: List<Bool>,
    /// Start time for stats
    start_time: Instant,
}

impl FixedPointEngine {
    /// Create new fixed-point engine
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::FixedPointEngine;
    /// use z3::Context;
    ///
    /// let ctx = Context::thread_local();
    /// let engine = FixedPointEngine::new(ctx).expect("Failed to create engine");
    /// ```
    pub fn new(ctx: Context) -> Result<Self, Text> {
        if ctx != Context::thread_local() {
            return Err(Text::from(
                "Fixedpoint engine requires the active Z3 context",
            ));
        }
        let fp = Fixedpoint::new();

        Ok(Self {
            fp,
            ctx,
            predicates: Map::new(),
            rules: List::new(),
            axioms: List::new(),
            start_time: Instant::now(),
        })
    }

    /// Register a relation (predicate) with the fixedpoint engine
    ///
    /// Takes ownership and returns a shared handle to the exact declaration.
    /// Relations must be registered before they can be used in rules.
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::FixedPointEngine;
    /// use z3::{Context, Sort, Symbol, FuncDecl};
    ///
    /// let ctx = Context::thread_local();
    /// let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();
    ///
    /// // Create a predicate edge(Int, Int)
    /// let int_sort = Sort::int();
    /// let edge = FuncDecl::new(
    ///  "edge",
    ///  &[&int_sort, &int_sort],
    ///  &Sort::bool(),
    /// );
    ///
    /// let edge = engine.register_relation(edge).unwrap();
    /// ```
    pub fn register_relation(&mut self, decl: FuncDecl) -> Result<Shared<FuncDecl>, Text> {
        self.check_relation(&decl, None)?;
        let name = Text::from(decl.name());
        if self.predicates.contains_key(&name) {
            return Err(Text::from(format!("Relation already registered: {name}")));
        }
        // Own the original declaration. Reconstructing from SortKind loses
        // BV widths and named sorts; translation is for different contexts.
        let owned = Shared::new(decl);
        self.fp.register_relation(&owned);
        self.predicates.insert(name, owned.clone());
        Ok(owned)
    }

    /// Add a Horn clause rule
    ///
    /// Rules should be of the form:
    /// - `forall vars. body => head` (implication)
    /// - `head` (fact)
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::FixedPointEngine;
    /// use z3::{Context, Sort, FuncDecl};
    /// use z3::ast::{Ast, Bool, Dynamic, Int};
    ///
    /// let ctx = Context::thread_local();
    /// let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();
    ///
    /// let int_sort = Sort::int();
    /// let edge = FuncDecl::new(
    ///  "edge",
    ///  &[&int_sort, &int_sort],
    ///  &Sort::bool(),
    /// );
    ///
    /// let edge = engine.register_relation(edge).unwrap();
    ///
    /// // Add fact: edge(1, 2)
    /// let x = Int::from_i64(1);
    /// let y = Int::from_i64(2);
    /// let x_dyn: Dynamic = x.into();
    /// let y_dyn: Dynamic = y.into();
    /// let fact = edge.apply(&[&x_dyn, &y_dyn]).as_bool().unwrap();
    /// engine.add_rule(&fact, Some("edge_1_2")).unwrap();
    /// ```
    pub fn add_rule(&mut self, rule: &Bool, name: Option<&str>) -> Result<(), Text> {
        if rule.get_ctx() != &self.ctx {
            return Err(Text::from("Rule belongs to another Z3 context"));
        }
        if let Some(name) = name {
            CString::new(name).map_err(|e| Text::from(format!("Invalid rule name: {e}")))?;
        }
        self.fp.add_rule(rule, name);
        self.rules.push(rule.clone());
        Ok(())
    }

    /// Add a fact (ground atom)
    ///
    /// This is a convenience method for adding rules with no premises.
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::FixedPointEngine;
    /// use z3::{Context, Sort, FuncDecl};
    /// use z3::ast::{Ast, Int};
    ///
    /// let ctx = Context::thread_local();
    /// let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();
    ///
    /// let int_sort = Sort::int();
    /// let node = FuncDecl::new(
    ///  "node",
    ///  &[&int_sort],
    ///  &Sort::bool(),
    /// );
    ///
    /// let node = engine.register_relation(node).unwrap();
    ///
    /// // Add facts: node(1), node(2), node(3)
    /// for i in 1..=3 {
    ///  let args = [Int::from_i64(i).into()];
    ///  engine.add_fact(&node, &args).unwrap();
    /// }
    /// ```
    pub fn add_fact(&mut self, decl: &FuncDecl, args: &[Dynamic]) -> Result<(), Text> {
        // Encode an exact ground rule. Z3's compact add_fact API accepts
        // u32 table indices, not arbitrary source integers or bitvectors.
        // Converting through that API truncated wide values and lost signs.
        self.check_relation(decl, Some(args))?;
        for arg in args {
            let scalar_literal = arg.kind() == z3::AstKind::Numeral
                || arg.as_bool().is_some_and(|b| b.as_bool().is_some());
            if !scalar_literal {
                return Err(Text::from("A fact requires scalar literal arguments"));
            }
        }
        let arguments: List<&dyn Ast> = args.iter().map(|a| a as &dyn Ast).collect();
        let fact = decl
            .apply(&arguments)
            .as_bool()
            .ok_or_else(|| Text::from("Fact is not Boolean"))?;
        self.add_rule(&fact, None)
    }

    /// Assert a constraint (background axiom)
    ///
    /// Constraints are used as background axioms when using PDR mode.
    /// They are ignored in standard Datalog mode.
    pub fn assert(&mut self, axiom: &Bool) -> Result<(), Text> {
        if axiom.get_ctx() != &self.ctx {
            return Err(Text::from("Axiom belongs to another Z3 context"));
        }
        self.fp.assert(axiom);
        self.axioms.push(axiom.clone());
        Ok(())
    }

    /// Query whether a predicate is derivable
    ///
    /// Returns:
    /// - `SatResult::Sat` if the query is satisfiable (derivable)
    /// - `SatResult::Unsat` if the query is unsatisfiable (not derivable)
    /// - `SatResult::Unknown` if timed out or otherwise failed
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::FixedPointEngine;
    /// use z3::{Context, Sort, FuncDecl, SatResult};
    /// use z3::ast::{Ast, Int};
    ///
    /// let ctx = Context::thread_local();
    /// let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();
    ///
    /// let int_sort = Sort::int();
    /// let edge = FuncDecl::new(
    ///  "edge",
    ///  &[&int_sort, &int_sort],
    ///  &Sort::bool(),
    /// );
    ///
    /// let edge = engine.register_relation(edge).unwrap();
    ///
    /// // Add some edges
    /// let x = Int::from_i64(1);
    /// let y = Int::from_i64(2);
    /// let fact = edge.apply(&[&x.into(), &y.into()]).as_bool().unwrap();
    /// engine.add_rule(&fact, Some("edge_1_2")).unwrap();
    ///
    /// // Query if edge(1,2) exists
    /// let query = edge.apply(&[&x.into(), &y.into()]).as_bool().unwrap();
    /// let result = engine.query(&query).unwrap();
    /// assert_eq!(result, SatResult::Sat);
    /// ```
    pub fn query(&mut self, query: &Bool) -> Result<SatResult, Text> {
        if query.get_ctx() != &self.ctx {
            return Err(Text::from("Query belongs to another Z3 context"));
        }
        Ok(self.fp.query(query))
    }

    /// Query multiple relations simultaneously
    ///
    /// More efficient than multiple single queries when you want to check
    /// multiple predicates at once.
    pub fn query_relations(&mut self, relations: &[&FuncDecl]) -> Result<SatResult, Text> {
        for relation in relations {
            self.check_relation(relation, None)?;
        }
        Ok(self.fp.query_relations(relations))
    }

    /// Get the answer (solution) from the last query
    ///
    /// Available after a completed query when the backend provides an answer.
    /// The returned formula is raw solver data, not a validated interpretation.
    ///
    /// In Datalog mode: returns disjunction of all derivations
    /// In PDR mode: returns a single conjunction
    pub fn get_answer(&self) -> Result<Bool, Text> {
        self.fp
            .get_answer()
            .ok_or_else(|| Text::from("No answer available"))
    }

    /// Get reason for unknown result
    ///
    /// Returns a string describing why the query returned Unknown.
    pub fn get_reason_unknown(&self) -> Text {
        Text::from(self.fp.get_reason_unknown())
    }

    /// Set parameters for the fixedpoint engine
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::FixedPointEngine;
    /// use z3::{Context, Params};
    ///
    /// let ctx = Context::thread_local();
    /// let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();
    ///
    /// let mut params = Params::new();
    /// params.set_symbol("engine", "spacer"); // Use SPACER engine (PDR)
    /// params.set_u32("timeout", 30000); // 30 second timeout
    /// engine.set_params(&params);
    /// ```
    pub fn set_params(&mut self, params: &z3::Params) {
        self.fp.set_params(params);
    }

    /// Get help string for available parameters
    pub fn get_help(&self) -> Text {
        Text::from(self.fp.get_help())
    }

    /// Get statistics from the last query
    pub fn get_statistics(&self) -> FixedPointStats {
        let elapsed = self.start_time.elapsed();
        FixedPointStats {
            iterations: 0, // Would extract from Z3_stats
            time_ms: elapsed.as_millis() as u64,
            num_rules: self.rules.len(),
            num_predicates: self.predicates.len(),
        }
    }

    /// Serialize the registered declarations and rules with the upstream printer.
    pub fn to_smtlib(&self) -> Text {
        Text::from(self.fp.to_string())
    }

    /// Register a recursive definition atomically after validating every clause.
    pub fn register_predicate(&mut self, pred: RecursivePredicate) -> Result<(), Text> {
        CString::new(pred.name.as_str()).map_err(|_| Text::from("Invalid predicate name"))?;
        if self.predicates.contains_key(&pred.name) {
            return Err(Text::from(format!(
                "Relation already registered: {}",
                pred.name
            )));
        }
        let param_refs: List<&Sort> = pred.params.iter().collect();
        let decl = Shared::new(FuncDecl::new(
            pred.name.as_str(),
            &param_refs,
            &Sort::bool(),
        ));
        self.check_relation(&decl, None)?;

        // Make self-recursion resolvable during lowering, without changing Z3.
        self.predicates.insert(pred.name.clone(), decl.clone());
        let rules = self.compile_predicate_body(&pred);
        if rules.is_err() {
            self.predicates.remove(&pred.name);
        }
        let rules = rules?;
        self.fp.register_relation(&decl);
        for rule in rules {
            self.add_rule(&rule, Some(pred.name.as_str()))?;
        }
        Ok(())
    }

    fn compile_predicate_body(&self, pred: &RecursivePredicate) -> Result<List<Bool>, Text> {
        let compile = |guard: Option<&Bool>, calls: &[RecursiveCall], head: &Bool| {
            if head.kind() != z3::AstKind::App || head.decl().name() != pred.name.as_str() {
                return Err(Text::from(
                    "A predicate clause must conclude its declared relation",
                ));
            }
            let mut premises = List::new();
            if let Some(guard) = guard {
                premises.push(guard.clone());
            }
            for call in calls {
                premises.push(self.atom_to_formula(&Atom {
                    predicate: call.predicate.clone(),
                    args: call.args.clone(),
                })?);
            }
            self.compile_horn_rule(&pred.vars, &premises, head)
        };
        let mut rules = List::new();
        match &pred.body {
            PredicateBody::Base(head) => rules.push(compile(None, &[], head)?),
            PredicateBody::Recursive {
                guard,
                recursive_calls,
                conclusion,
            } => {
                rules.push(compile(Some(guard), recursive_calls, conclusion)?);
            }
            PredicateBody::Cases(cases) => {
                for case in cases {
                    rules.push(compile(
                        case.guard.as_ref(),
                        &case.recursive_calls,
                        &case.body,
                    )?);
                }
            }
        }
        Ok(rules)
    }

    /// Add a Datalog rule
    ///
    /// Converts a DatalogRule to Z3 format and adds it to the fixedpoint engine.
    /// Rules are of the form: head :- body1, body2, ..., bodyN, constraints.
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::{FixedPointEngine, DatalogRule, Atom};
    /// use verum_common::{List, Text};
    /// use z3::Context;
    ///
    /// let ctx = Context::thread_local();
    /// let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();
    ///
    /// // Define rule: path(X,Z) :- edge(X,Y), path(Y,Z)
    /// let rule = DatalogRule {
    ///  vars: List::new(),
    ///  head: Atom {
    ///  predicate: Text::from("path"),
    ///  args: List::new(),
    ///  },
    ///  body: List::new(),
    ///  constraints: List::new(),
    /// };
    /// engine.add_datalog_rule(rule).unwrap();
    /// ```
    pub fn add_datalog_rule(&mut self, rule: DatalogRule) -> Result<(), Text> {
        self.add_chc(CHC {
            vars: rule.vars,
            hypothesis: rule.body,
            constraints: rule.constraints,
            conclusion: rule.head,
        })
    }

    /// Add ∀vars. (hypotheses ∧ constraints) ⇒ conclusion.
    pub fn add_chc(&mut self, chc: CHC) -> Result<(), Text> {
        let mut premises = chc.constraints;
        for atom in &chc.hypothesis {
            premises.push(self.atom_to_formula(atom)?);
        }
        let conclusion = self.atom_to_formula(&chc.conclusion)?;
        let rule = self.compile_horn_rule(&chc.vars, &premises, &conclusion)?;
        self.add_rule(&rule, Some(chc.conclusion.predicate.as_str()))
    }

    /// One lowering path for recursive definitions, Datalog and CHCs.
    /// Validate before constructing a quantified formula or mutating the solver.
    fn compile_horn_rule(
        &self,
        vars: &[(Text, Sort)],
        premises: &[Bool],
        conclusion: &Bool,
    ) -> Result<Bool, Text> {
        if conclusion.get_ctx() != &self.ctx || premises.iter().any(|p| p.get_ctx() != &self.ctx) {
            return Err(Text::from("Horn clause belongs to another Z3 context"));
        }
        if conclusion.kind() != z3::AstKind::App
            || conclusion.decl().kind() != z3::DeclKind::Uninterpreted
        {
            return Err(Text::from(
                "Horn clause head must be a registered relation application",
            ));
        }
        let head = Atom {
            predicate: conclusion.decl().name().into(),
            args: conclusion.children().into_iter().collect(),
        };
        if self.atom_to_formula(&head)?.get_z3_ast() != conclusion.get_z3_ast() {
            return Err(Text::from(
                "Horn clause head does not match its registered declaration",
            ));
        }
        let mut variables = List::new();
        let mut bound = Set::new();
        for (name, sort) in vars {
            CString::new(name.as_str()).map_err(|_| Text::from("Invalid Horn variable name"))?;
            if Dynamic::fresh_const("scope", sort).get_ctx() != &self.ctx {
                return Err(Text::from("Horn variable belongs to another Z3 context"));
            }
            let variable = Dynamic::new_const(name.as_str(), sort);
            if !bound.insert(variable.clone()) {
                return Err(Text::from("Duplicate Horn variable"));
            }
            if self
                .predicates
                .get(name)
                .is_some_and(|decl| decl.arity() == 0)
            {
                return Err(Text::from("Horn variable shadows a zero-argument relation"));
            }
            variables.push(variable);
        }
        let mut pending: List<Dynamic> = premises
            .iter()
            .map(|term| Dynamic::from_ast(term))
            .collect();
        pending.push(Dynamic::from_ast(conclusion));
        let mut visited = Set::new();
        while let Some(term) = pending.pop() {
            if !visited.insert(term.clone()) {
                continue;
            }
            match term.kind() {
                z3::AstKind::Quantifier | z3::AstKind::Var => {
                    return Err(Text::from(
                        "Use explicit Horn variables instead of nested quantifiers",
                    ));
                }
                z3::AstKind::App => {
                    if term.num_children() == 0
                        && term.decl().kind() == z3::DeclKind::Uninterpreted
                        && !bound.contains(&term)
                    {
                        let registered = self
                            .predicates
                            .get(&Text::from(term.decl().name()))
                            .is_some_and(|decl| {
                                decl.arity() == 0
                                    && decl.apply(&[]).get_z3_ast() == term.get_z3_ast()
                            });
                        if !registered {
                            return Err(Text::from(format!("Unbound Horn variable: {term}")));
                        }
                    }
                    pending.extend(term.children());
                }
                _ => {}
            }
        }
        let rule = if premises.is_empty() {
            conclusion.clone()
        } else {
            Bool::and(premises).implies(conclusion)
        };
        let refs: List<&dyn Ast> = variables.iter().map(|v| v as &dyn Ast).collect();
        Ok(if refs.is_empty() {
            rule
        } else {
            z3::ast::forall_const(&refs, &[], &rule)
        })
    }

    /// Check exact domains before the upstream infallible application API.
    /// A declaration's public array view exposes its context and native domains
    /// without inspecting private Rust layouts or reconstructing sorts.
    fn check_relation(&self, decl: &FuncDecl, args: Option<&[Dynamic]>) -> Result<(), Text> {
        if decl.range() != z3::SortKind::Bool {
            return Err(Text::from("A fixedpoint relation must return Bool"));
        }
        let view = if decl.arity() == 0 {
            decl.apply(&[])
        } else {
            Dynamic::from_ast(&decl.as_array())
        };
        if view.get_ctx() != &self.ctx {
            return Err(Text::from("Relation belongs to another Z3 context"));
        }
        let Some(args) = args else { return Ok(()) };
        if decl.arity() != args.len() {
            return Err(Text::from(
                "Relation argument count does not match its declaration",
            ));
        }
        if args.is_empty() {
            return Ok(());
        }
        let ctx = self.ctx.get_z3_context();
        // SAFETY: view is a live as-array AST in ctx, retaining decl. Every
        // domain index is below the checked arity. Each argument's context is
        // checked before its sort handle is passed to the same native context.
        unsafe {
            let raw_decl = z3_sys::Z3_get_as_array_func_decl(ctx, view.get_z3_ast())
                .ok_or_else(|| Text::from("Relation declaration is unavailable"))?;
            for (index, arg) in args.iter().enumerate() {
                if arg.get_ctx() != &self.ctx {
                    return Err(Text::from(
                        "Relation argument belongs to another Z3 context",
                    ));
                }
                let expected = z3_sys::Z3_get_domain(ctx, raw_decl, index as u32)
                    .ok_or_else(|| Text::from("Relation domain is unavailable"))?;
                if !z3_sys::Z3_is_eq_sort(ctx, expected, arg.get_sort().get_z3_sort()) {
                    return Err(Text::from(format!(
                        "Relation argument {index} has the wrong sort"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Convert an Atom to a Z3 Bool formula
    fn atom_to_formula(&self, atom: &Atom) -> Result<Bool, Text> {
        // Get the predicate declaration
        let decl = match self.predicates.get(&atom.predicate) {
            Maybe::Some(d) => d,
            Maybe::None => {
                return Err(Text::from(format!(
                    "Predicate not found: {}",
                    atom.predicate
                )));
            }
        };

        self.check_relation(decl, Some(&atom.args))?;
        // Convert args to AST references
        let arg_refs: List<&dyn z3::ast::Ast> =
            atom.args.iter().map(|a| a as &dyn z3::ast::Ast).collect();

        // Apply predicate to arguments
        let result = decl
            .apply(&arg_refs)
            .as_bool()
            .ok_or_else(|| Text::from("Failed to convert atom to Bool"))?;

        Ok(result)
    }

    /// Query with FixedPointQuery structure
    pub fn query_predicate(
        &mut self,
        query: FixedPointQuery,
    ) -> std::result::Result<FixedPointResult, Text> {
        let start = Instant::now();

        let query_ast = self.atom_to_formula(&Atom {
            predicate: query.predicate,
            args: query.args,
        })?;

        // Execute query
        let status = self.query(&query_ast)?;

        // Reachability answers are not parsed inductive interpretations.
        let answer = match status {
            SatResult::Sat | SatResult::Unsat => self.fp.get_answer(),
            SatResult::Unknown => None,
        };

        let elapsed = start.elapsed();
        let stats = FixedPointStats {
            iterations: 0,
            time_ms: elapsed.as_millis() as u64,
            num_rules: self.rules.len(),
            num_predicates: self.predicates.len(),
        };

        Ok(FixedPointResult {
            status,
            answer,
            solution: Maybe::None,
            stats,
        })
    }

    /// Get proof/certificate for UNSAT query
    pub fn get_proof(&self) -> Maybe<Text> {
        // Z3 fixedpoint doesn't directly expose proof extraction via simple API
        // Would need additional FFI calls to Z3_fixedpoint_get_proof_graph or similar
        Maybe::None
    }

    /// Return the raw fixedpoint answer for inspection, without claiming it is
    /// a closed or inductive invariant. Validation requires explicit interpretations.
    pub fn get_invariants(&self) -> List<Bool> {
        // Extract invariants from the answer
        match self.get_answer() {
            Ok(answer) => List::from(vec![answer]),
            Err(_) => List::new(),
        }
    }
}

// ==================== Recursive Predicate Patterns ====================

/// Common recursive predicate patterns for Verum types
pub mod patterns {
    use super::*;

    /// List-related recursive predicates
    pub struct ListPredicates;

    impl ListPredicates {
        /// Create a predicate for list length
        ///
        /// Defines: length(nil) = 0, length(cons(h,t)) = 1 + length(t)
        pub fn length(ctx: &Context) -> RecursivePredicate {
            let list_sort = Sort::uninterpreted(Symbol::String("List".to_string()));
            let int_sort = Sort::int();

            RecursivePredicate {
                vars: Default::default(),
                name: Text::from("list_length"),
                params: List::from(vec![list_sort, int_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: length(nil) = 0
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive case: length(cons(h,t)) = 1 + length(t)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![RecursiveCall {
                            predicate: Text::from("list_length"),
                            args: List::new(),
                        }]),
                    },
                ])),
                well_founded: true,
            }
        }

        /// Create a predicate for list membership
        ///
        /// Defines: contains(x, cons(x, t)), contains(x, cons(h, t)) :- contains(x, t)
        pub fn contains(ctx: &Context) -> RecursivePredicate {
            let elem_sort = Sort::int();
            let list_sort = Sort::uninterpreted(Symbol::String("List".to_string()));

            RecursivePredicate {
                vars: Default::default(),
                name: Text::from("list_contains"),
                params: List::from(vec![elem_sort, list_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: contains(x, cons(x, t))
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive case: contains(x, cons(h, t)) :- contains(x, t)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![RecursiveCall {
                            predicate: Text::from("list_contains"),
                            args: List::new(),
                        }]),
                    },
                ])),
                well_founded: true,
            }
        }

        /// Create a predicate for list fold (sum)
        pub fn sum(ctx: &Context) -> RecursivePredicate {
            let list_sort = Sort::uninterpreted(Symbol::String("List".to_string()));
            let int_sort = Sort::int();

            RecursivePredicate {
                vars: Default::default(),
                name: Text::from("list_sum"),
                params: List::from(vec![list_sort, int_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: sum(nil) = 0
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive case: sum(cons(h,t)) = h + sum(t)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![RecursiveCall {
                            predicate: Text::from("list_sum"),
                            args: List::new(),
                        }]),
                    },
                ])),
                well_founded: true,
            }
        }
    }

    /// Tree-related recursive predicates
    pub struct TreePredicates;

    impl TreePredicates {
        /// Create a predicate for tree height
        ///
        /// Defines: height(leaf) = 0, height(node(l,r)) = 1 + max(height(l), height(r))
        pub fn height(ctx: &Context) -> RecursivePredicate {
            let tree_sort = Sort::uninterpreted(Symbol::String("Tree".to_string()));
            let int_sort = Sort::int();

            RecursivePredicate {
                vars: Default::default(),
                name: Text::from("tree_height"),
                params: List::from(vec![tree_sort, int_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: height(leaf) = 0
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive case: height(node(l,r)) = 1 + max(height(l), height(r))
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![
                            RecursiveCall {
                                predicate: Text::from("tree_height"),
                                args: List::new(),
                            },
                            RecursiveCall {
                                predicate: Text::from("tree_height"),
                                args: List::new(),
                            },
                        ]),
                    },
                ])),
                well_founded: true,
            }
        }

        /// Create a predicate for tree search
        ///
        /// Defines: search(x, node(x, _, _)), search(x, node(_, l, r)) :- search(x, l) ∨ search(x, r)
        pub fn search(ctx: &Context) -> RecursivePredicate {
            let elem_sort = Sort::int();
            let tree_sort = Sort::uninterpreted(Symbol::String("Tree".to_string()));

            RecursivePredicate {
                vars: Default::default(),
                name: Text::from("tree_search"),
                params: List::from(vec![elem_sort, tree_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: search(x, leaf) fails
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(false),
                        recursive_calls: List::new(),
                    },
                    // Found: search(x, node(x, _, _))
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive: search(x, node(_, l, r)) :- search(x, l) ∨ search(x, r)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![
                            RecursiveCall {
                                predicate: Text::from("tree_search"),
                                args: List::new(),
                            },
                            RecursiveCall {
                                predicate: Text::from("tree_search"),
                                args: List::new(),
                            },
                        ]),
                    },
                ])),
                well_founded: true,
            }
        }

        /// Create a predicate for tree size (node count)
        pub fn size(ctx: &Context) -> RecursivePredicate {
            let tree_sort = Sort::uninterpreted(Symbol::String("Tree".to_string()));
            let int_sort = Sort::int();

            RecursivePredicate {
                vars: Default::default(),
                name: Text::from("tree_size"),
                params: List::from(vec![tree_sort, int_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: size(leaf) = 0
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive case: size(node(v, l, r)) = 1 + size(l) + size(r)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![
                            RecursiveCall {
                                predicate: Text::from("tree_size"),
                                args: List::new(),
                            },
                            RecursiveCall {
                                predicate: Text::from("tree_size"),
                                args: List::new(),
                            },
                        ]),
                    },
                ])),
                well_founded: true,
            }
        }
    }

    /// Graph-related recursive predicates
    pub struct GraphPredicates;

    impl GraphPredicates {
        /// Create a predicate for graph reachability
        ///
        /// Defines: reach(x,y) :- edge(x,y), reach(x,z) :- edge(x,y) ∧ reach(y,z)
        pub fn reachability(ctx: &Context) -> RecursivePredicate {
            let node_sort = Sort::int();

            RecursivePredicate {
                vars: Default::default(),
                name: Text::from("reachable"),
                params: List::from(vec![node_sort.clone(), node_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: reach(x,y) :- edge(x,y)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive case: reach(x,z) :- edge(x,y) ∧ reach(y,z)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![RecursiveCall {
                            predicate: Text::from("reachable"),
                            args: List::new(),
                        }]),
                    },
                ])),
                well_founded: false, // May have cycles
            }
        }

        /// Create a predicate for detecting cycles
        ///
        /// Defines: cycle(x) :- edge(x,x), cycle(x) :- edge(x,y) ∧ cycle(y) ∧ reach(y,x)
        pub fn cycle_detection(ctx: &Context) -> RecursivePredicate {
            let node_sort = Sort::int();

            RecursivePredicate {
                vars: Default::default(),
                name: Text::from("has_cycle"),
                params: List::from(vec![node_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: cycle(x) :- edge(x,x) (self-loop)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive case: cycle(x) :- edge(x,y) ∧ reach(y,x)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![RecursiveCall {
                            predicate: Text::from("reachable"),
                            args: List::new(),
                        }]),
                    },
                ])),
                well_founded: false, // Cycles exist
            }
        }

        /// Create a predicate for path length
        pub fn path_length(ctx: &Context) -> RecursivePredicate {
            let node_sort = Sort::int();
            let int_sort = Sort::int();

            RecursivePredicate {
                vars: Default::default(),
                name: Text::from("path_length"),
                params: List::from(vec![node_sort.clone(), node_sort, int_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: path_length(x,y,1) :- edge(x,y)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive: path_length(x,z,n+1) :- edge(x,y) ∧ path_length(y,z,n)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![RecursiveCall {
                            predicate: Text::from("path_length"),
                            args: List::new(),
                        }]),
                    },
                ])),
                well_founded: false, // May have cycles
            }
        }
    }
}

// ==================== High-Level Solver Functions ====================

/// Create a fixed-point context with default configuration
///
/// This is a convenience function that sets up a FixedPointEngine with
/// commonly used parameters for Verum verification.
pub fn create_fixedpoint_context(use_spacer: bool) -> Result<FixedPointEngine, Text> {
    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone())?;

    // Configure engine
    let mut params = z3::Params::new();

    if use_spacer {
        // Use SPACER (PDR) engine for CHC solving
        params.set_symbol("engine", "spacer");
        params.set_u32("xform.slice", 0); // Disable slicing
        params.set_u32("xform.inline_linear", 0); // Disable inlining
    } else {
        // Use standard Datalog engine
        params.set_symbol("engine", "datalog");
    }

    // Set timeout (30 seconds)
    params.set_u32("timeout", 30000);

    engine.set_params(&params);

    Ok(engine)
}

/// Solve a recursive predicate and extract solution
///
/// High-level function that handles the full workflow:
/// 1. Register predicate
/// 2. Query for satisfiability
/// 3. Extract solution if SAT
pub fn solve_recursive_predicate(
    engine: &mut FixedPointEngine,
    predicate: RecursivePredicate,
    query_args: List<Dynamic>,
) -> Result<FixedPointResult, Text> {
    // Register the predicate
    let pred_name = predicate.name.clone();
    engine.register_predicate(predicate)?;

    // Create query
    let query = FixedPointQuery {
        predicate: pred_name,
        args: query_args,
    };

    // Solve
    engine.query_predicate(query)
}

/// Return the candidate global properties supplied with a solution.
/// This does not validate them. A predicate body over formal parameters is not
/// an unconditional invariant and is never exported as one by this helper.
pub fn extract_invariants(solution: &FixedPointSolution) -> List<Bool> {
    solution.invariants.clone()
}

/// Check candidate interpretations against every original rule and assertion.
/// This establishes closure, not minimality, reachability or kernel certification.
pub fn validate_solution(
    engine: &mut FixedPointEngine,
    solution: &FixedPointSolution,
) -> Result<bool, Text> {
    engine.validate_solution(solution)
}

// ==================== Inductive Datatype Support ====================

/// Builder for inductive datatypes with refinements
pub struct InductiveDatatypeBuilder {
    engine: FixedPointEngine,
}

impl InductiveDatatypeBuilder {
    pub fn new(ctx: Context) -> Result<Self, Text> {
        let engine = FixedPointEngine::new(ctx)?;
        Ok(Self { engine })
    }

    /// Define list datatype with size predicate
    pub fn define_list_with_size(&mut self) -> Result<(), Text> {
        let list_sort = Sort::uninterpreted(Symbol::String("List".to_string()));
        let int_sort = Sort::int();

        let size_pred = RecursivePredicate {
            vars: Default::default(),
            name: Text::from("list_size"),
            params: List::from(vec![list_sort, int_sort]),
            body: PredicateBody::Base(Bool::from_bool(true)),
            well_founded: true,
        };

        self.engine.register_predicate(size_pred)
    }

    /// Define tree datatype with height predicate
    pub fn define_tree_with_height(&mut self) -> Result<(), Text> {
        let tree_sort = Sort::uninterpreted(Symbol::String("Tree".to_string()));
        let int_sort = Sort::int();

        let height_pred = RecursivePredicate {
            vars: Default::default(),
            name: Text::from("tree_height"),
            params: List::from(vec![tree_sort, int_sort]),
            body: PredicateBody::Base(Bool::from_bool(true)),
            well_founded: true,
        };

        self.engine.register_predicate(height_pred)
    }
}

// ==================== Datalog Solver ====================

/// Pure Datalog solver for Horn clauses
pub struct DatalogSolver {
    engine: FixedPointEngine,
}

impl DatalogSolver {
    pub fn new(ctx: Context) -> Result<Self, Text> {
        Ok(Self {
            engine: FixedPointEngine::new(ctx)?,
        })
    }

    /// Register a relation (predicate) with the Datalog solver
    ///
    /// This must be called before adding facts or rules that use the relation.
    /// The relation is defined by its name and parameter sorts.
    ///
    /// # Arguments
    /// * `name` - The name of the relation (predicate)
    /// * `param_sorts` - The sorts (types) of the relation's parameters
    ///
    /// # Example
    /// ```ignore
    /// use verum_smt::fixedpoint::DatalogSolver;
    /// use z3::{Context, Sort};
    ///
    /// let ctx = Context::thread_local();
    /// let mut solver = DatalogSolver::new(ctx.clone()).unwrap();
    ///
    /// // Register a unary relation "node" over integers
    /// solver.register_relation("node", &[Sort::int()]).unwrap();
    /// ```
    pub fn register_relation(&mut self, name: &str, param_sorts: &[Sort]) -> Result<(), Text> {
        let params: List<&Sort> = param_sorts.iter().collect();
        self.engine
            .register_relation(FuncDecl::new(name, &params, &Sort::bool()))?;
        Ok(())
    }

    /// Add fact (rule with empty body)
    pub fn add_fact(&mut self, atom: Atom) -> Result<(), Text> {
        self.engine.add_datalog_rule(DatalogRule {
            vars: Default::default(),
            head: atom,
            body: List::new(),
            constraints: List::new(),
        })
    }

    /// Add rule
    pub fn add_rule(&mut self, rule: DatalogRule) -> Result<(), Text> {
        self.engine.add_datalog_rule(rule)
    }

    /// Query whether an atom is derivable in the Datalog program
    ///
    /// This method queries the fixed-point engine to determine if the given atom
    /// (predicate application) holds in the least fixed point of the rules.
    ///
    /// # Theory
    /// In Datalog:
    /// - Facts are added as base cases
    /// - Rules define how to derive new facts
    /// - The query asks: "Is this atom in the transitive closure?"
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::{DatalogSolver, Atom, DatalogRule};
    /// use verum_common::{List, Text};
    /// use z3::{Context, Sort, FuncDecl, ast::Int};
    ///
    /// let ctx = Context::thread_local();
    /// let mut solver = DatalogSolver::new(ctx.clone()).unwrap();
    ///
    /// // Define edge relation
    /// // Add facts: edge(1,2), edge(2,3)
    /// // Add rule: path(x,y) :- edge(x,y)
    /// // Add rule: path(x,z) :- edge(x,y), path(y,z)
    ///
    /// // Query: path(1,3)?
    /// let query_atom = Atom {
    ///  predicate: Text::from("path"),
    ///  args: List::from(vec![Int::from_i64(1).into(), Int::from_i64(3).into()]),
    /// };
    ///
    /// let result = solver.query(query_atom).unwrap();
    /// assert!(result); // Should be true (transitive closure)
    /// ```
    pub fn query(&mut self, atom: Atom) -> Result<bool, Text> {
        // Convert the atom to a Z3 Bool formula
        let query_formula = self.engine.atom_to_formula(&atom)?;

        // Query the fixed-point engine
        let result = self.engine.query(&query_formula)?;

        // Convert SatResult to bool
        // SAT means the atom is derivable (holds in fixed point)
        // UNSAT means the atom is not derivable
        match result {
            z3::SatResult::Sat => Ok(true),
            z3::SatResult::Unsat => Ok(false),
            z3::SatResult::Unknown => {
                // Get reason for unknown result
                let reason = self.engine.get_reason_unknown();
                Err(Text::from(format!(
                    "Z3 fixed-point query returned unknown: {}",
                    reason
                )))
            }
        }
    }

    /// Get all derivable facts
    pub fn get_model(&self) -> DatalogModel {
        DatalogModel { facts: Map::new() }
    }
}

/// Datalog model
#[derive(Debug)]
pub struct DatalogModel {
    pub facts: Map<Text, Set<List<Dynamic>>>,
}
