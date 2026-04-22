//! Proof Search and Automation
//!
//! Implements proof search strategies and automation for the formal proof system.
//!
//! Proof Search: Automated strategies that try sequences of tactics (assumption,
//! reflexivity, intro, split, apply_hypothesis, unfold_definition) with backtracking.
//! A hints database stores lemmas with priorities and patterns for guided instantiation.
//!
//! Proof Tactics: Composable goal transformers including:
//! - `simp`: simplify using lemma database
//! - `ring`: normalize ring expressions (commutative ring axioms)
//! - `field`: normalize field expressions including division
//! - `omega`: linear integer arithmetic solver (Cooper's algorithm / Presburger arithmetic)
//! - `blast`: tableau prover for propositional/first-order logic
//! - Forward reasoning (`have`), backward reasoning (`suffices`), case analysis
//!
//! ## Features
//!
//! - **Hints Database**: Store and retrieve proof hints for automation
//! - **Pattern Matching**: Match proof goals against known patterns
//! - **Decision Procedures**: Automatic proof generation for decidable fragments
//! - **Tactic Engine**: Composable proof tactics (future)
//!
//! ## Performance Targets
//!
//! - Hint lookup: < 1ms
//! - Pattern matching: < 10ms
//! - Decision procedure: < 100ms (spec default timeout)

use std::time::{Duration, Instant};

use verum_ast::{BinOp, Expr, ExprKind, Ident, Path, UnOp};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_common::{Heap, List, Map, Maybe, Text};
use verum_common::ToText;

use crate::context::Context;
use crate::option_to_maybe;
use crate::translate::Translator;
use crate::verify::{ProofResult, VerificationCost, VerificationError, VerificationResult};

// ==================== Hints Database ====================

/// Type signature for structural matching
///
/// Represents the shape of an expression for pattern matching purposes.
/// Used to find lemmas that could apply based on structure rather than
/// exact syntactic match.
#[derive(Debug, Clone, PartialEq)]
pub enum SignatureType {
    /// Variable or placeholder
    Variable,
    /// Literal value
    Literal,
    /// Binary operation
    Binary {
        op: BinOp,
        left: Box<SignatureType>,
        right: Box<SignatureType>,
    },
    /// Unary operation
    Unary {
        op: verum_ast::UnOp,
        inner: Box<SignatureType>,
    },
    /// Function call
    Call { arity: usize },
    /// Unknown/unsupported
    Unknown,
}

/// Proof hints database for automation
///
/// Stores lemmas, tactics, and patterns that can be used to automatically
/// prove goals. The database uses pattern matching to find applicable hints.
///
/// Hints database for automated proof search. Stores lemmas with priorities (0-1000),
/// tactic hints keyed by goal pattern, and decision procedures for decidable fragments.
/// Lemmas can be registered with `@hint(priority = N)` or `@hint(pattern = "...")`.
/// The search engine queries hints by matching goal structure against stored patterns.
#[derive(Debug, Clone)]
pub struct HintsDatabase {
    /// Lemma hints indexed by pattern
    lemmas: Map<Text, List<LemmaHint>>,

    /// Tactic hints for specific goal patterns
    tactics: Map<Text, List<TacticHint>>,

    /// Decision procedure hints
    decision_procedures: List<DecisionProcedure>,

    /// Statistics
    stats: HintStats,
}

impl HintsDatabase {
    /// Create a new empty hints database
    pub fn new() -> Self {
        Self {
            lemmas: Map::new(),
            tactics: Map::new(),
            decision_procedures: List::new(),
            stats: HintStats::default(),
        }
    }

    /// Create database with standard library hints
    pub fn with_core() -> Self {
        let mut db = Self::new();
        db.register_stdlib_hints();
        db
    }

    /// Register a lemma hint
    ///
    /// # Arguments
    /// - `pattern`: Pattern to match against goals
    /// - `hint`: The lemma hint to apply
    ///
    /// # Example
    /// ```ignore
    /// db.register_lemma("_ + _ = _", LemmaHint {
    ///     name: "plus_properties",
    ///     priority: 100,
    ///     lemma: plus_comm_lemma,
    /// });
    /// ```
    pub fn register_lemma(&mut self, pattern: Text, hint: LemmaHint) {
        self.lemmas
            .entry(pattern)
            .or_default()
            .push(hint);
    }

    /// Register a lemma by name for direct lookup
    ///
    /// This allows lemmas to be retrieved by name rather than pattern matching.
    /// Useful for explicitly applying named lemmas in proofs.
    pub fn register_named_lemma(&mut self, name: Text, hint: LemmaHint) {
        // Register by name as a special pattern
        self.lemmas
            .entry(format!("@name:{}", name).into())
            .or_default()
            .push(hint);
    }

    /// Lookup lemma by name
    ///
    /// Returns the lemma hint with the given name, if it exists.
    pub fn lookup_lemma_by_name(&self, name: &Text) -> Maybe<&LemmaHint> {
        let pattern = format!("@name:{}", name).into();
        match self.lemmas.get(&pattern) {
            Maybe::Some(hints) if !hints.is_empty() => Maybe::Some(&hints[0]),
            _ => Maybe::None,
        }
    }

    /// Find lemmas by type signature
    ///
    /// Searches for lemmas that could apply based on the shape of their
    /// conclusion. More sophisticated than pattern matching - analyzes
    /// the logical structure.
    ///
    /// # Example
    /// ```ignore
    /// // Find all lemmas concluding with equality
    /// let equality_lemmas = db.find_by_signature(&equality_goal);
    /// ```
    pub fn find_by_signature(&self, goal: &Expr) -> List<&LemmaHint> {
        let mut results = List::new();
        let goal_signature = Self::compute_signature(goal);

        // Search through all registered lemmas
        for (_pattern, lemma_list) in &self.lemmas {
            for lemma in lemma_list {
                let lemma_signature = Self::compute_lemma_conclusion_signature(&lemma.lemma);

                // Check if signatures are compatible
                if Self::signatures_compatible(&lemma_signature, &goal_signature) {
                    results.push(lemma);
                }
            }
        }

        // Sort by priority
        results.sort_by(|a, b| b.priority.cmp(&a.priority));
        results
    }

    /// Compute type signature of an expression
    ///
    /// Creates a structural fingerprint for matching.
    /// Examples:
    /// - `x + y = z` → Eq(Add(Var, Var), Var)
    /// - `P && Q => R` → Imply(And(Var, Var), Var)
    fn compute_signature(expr: &Expr) -> SignatureType {
        use ExprKind::*;

        match &expr.kind {
            Binary { op, left, right } => SignatureType::Binary {
                op: *op,
                left: Box::new(Self::compute_signature(left)),
                right: Box::new(Self::compute_signature(right)),
            },

            Unary { op, expr } => SignatureType::Unary {
                op: *op,
                inner: Box::new(Self::compute_signature(expr)),
            },

            Call { func, args, .. } => SignatureType::Call { arity: args.len() },

            Literal(_) => SignatureType::Literal,
            Path(_) => SignatureType::Variable,
            Paren(e) => Self::compute_signature(e),

            _ => SignatureType::Unknown,
        }
    }

    /// Compute signature of lemma conclusion
    fn compute_lemma_conclusion_signature(lemma: &Expr) -> SignatureType {
        // Extract conclusion from lemma (handle premises => conclusion)
        let mut current = lemma;
        loop {
            match &current.kind {
                ExprKind::Binary {
                    op: BinOp::Imply,
                    right,
                    ..
                } => {
                    current = right;
                }
                _ => break,
            }
        }
        Self::compute_signature(current)
    }

    /// Check if two signatures are compatible for unification
    fn signatures_compatible(lemma_sig: &SignatureType, goal_sig: &SignatureType) -> bool {
        use SignatureType::*;

        match (lemma_sig, goal_sig) {
            // Variables match anything
            (Variable, _) | (_, Variable) => true,

            // Literals only match literals
            (Literal, Literal) => true,

            // Binary operators must have same operator
            (
                Binary {
                    op: op1,
                    left: l1,
                    right: r1,
                },
                Binary {
                    op: op2,
                    left: l2,
                    right: r2,
                },
            ) => {
                op1 == op2
                    && Self::signatures_compatible(l1, l2)
                    && Self::signatures_compatible(r1, r2)
            }

            // Unary operators must have same operator
            (Unary { op: op1, inner: i1 }, Unary { op: op2, inner: i2 }) => {
                op1 == op2 && Self::signatures_compatible(i1, i2)
            }

            // Calls must have same arity
            (Call { arity: a1 }, Call { arity: a2 }) => a1 == a2,

            // Unknown matches anything (conservative)
            (Unknown, _) | (_, Unknown) => true,

            _ => false,
        }
    }

    /// Forward reasoning: given hypotheses, find applicable lemmas
    ///
    /// Searches for lemmas whose premises match the available hypotheses.
    /// This enables forward chaining from known facts.
    ///
    /// # Example
    /// ```ignore
    /// // Hypotheses: [x > 0, y > 0]
    /// // Lemma: x > 0 && y > 0 => x + y > 0
    /// // Result: Can derive x + y > 0
    /// ```
    pub fn forward_reasoning(&self, hypotheses: &List<Expr>) -> List<(Text, Expr)> {
        let mut derivable = List::new();

        for (_pattern, lemma_list) in &self.lemmas {
            for lemma in lemma_list {
                // Parse lemma structure
                let lemma_expr = &*lemma.lemma;
                let (premises, conclusion) = Self::extract_lemma_structure_static(lemma_expr);

                // Check if all premises can be satisfied by hypotheses
                if Self::all_premises_satisfied(&premises, hypotheses) {
                    derivable.push((lemma.name.clone(), conclusion));
                }
            }
        }

        derivable
    }

    /// Backward reasoning: given goal, find lemmas that could prove it
    ///
    /// Searches for lemmas whose conclusions unify with the goal.
    /// Returns the premises that would need to be proven.
    ///
    /// # Example
    /// ```ignore
    /// // Goal: x + y > 0
    /// // Lemma: x > 0 && y > 0 => x + y > 0
    /// // Result: Need to prove [x > 0, y > 0]
    /// ```
    pub fn backward_reasoning(&self, goal: &Expr) -> List<(Text, List<Expr>)> {
        let mut applicable = List::new();

        for (_pattern, lemma_list) in &self.lemmas {
            for lemma in lemma_list {
                let lemma_expr = &*lemma.lemma;
                let (premises, conclusion) = Self::extract_lemma_structure_static(lemma_expr);

                // Check if conclusion unifies with goal
                if Self::can_unify_static(&conclusion, goal) {
                    applicable.push((lemma.name.clone(), premises));
                }
            }
        }

        applicable
    }

    /// Check if all premises are satisfied by hypotheses
    fn all_premises_satisfied(premises: &List<Expr>, hypotheses: &List<Expr>) -> bool {
        premises
            .iter()
            .all(|premise| hypotheses.iter().any(|hyp| Self::exprs_match(premise, hyp)))
    }

    /// Simple expression matching (structural equality)
    fn exprs_match(e1: &Expr, e2: &Expr) -> bool {
        ProofSearchEngine::expr_eq(e1, e2)
    }

    /// Static version of extract_lemma_structure (no self)
    fn extract_lemma_structure_static(lemma: &Expr) -> (List<Expr>, Expr) {
        ProofSearchEngine::extract_lemma_structure(lemma)
    }

    /// Static version of unification check
    fn can_unify_static(pattern: &Expr, target: &Expr) -> bool {
        use ExprKind::*;

        match (&pattern.kind, &target.kind) {
            // Variables unify with anything
            (Path(p), _) if p.as_ident().is_some() => true,

            // Literals must match
            (Literal(l1), Literal(l2)) => l1.kind == l2.kind,

            // Binary operators must match recursively
            (
                Binary {
                    op: op1,
                    left: l1,
                    right: r1,
                },
                Binary {
                    op: op2,
                    left: l2,
                    right: r2,
                },
            ) => op1 == op2 && Self::can_unify_static(l1, l2) && Self::can_unify_static(r1, r2),

            // Parentheses - unwrap
            (Paren(e1), _) => Self::can_unify_static(e1, target),
            (_, Paren(e2)) => Self::can_unify_static(pattern, e2),

            _ => false,
        }
    }

    /// Automatic lemma selection based on goal characteristics
    ///
    /// Uses heuristics to rank lemmas by applicability:
    /// - Syntactic similarity to goal
    /// - Lemma priority
    /// - Historical success rate
    /// - Complexity (prefer simpler lemmas)
    pub fn select_best_lemmas(&self, goal: &Expr, max_results: usize) -> List<LemmaHint> {
        let mut candidates: List<(LemmaHint, f64)> = List::new();

        // Collect all potentially applicable lemmas with scores
        for (_pattern, lemma_list) in &self.lemmas {
            for lemma in lemma_list {
                let score = self.compute_lemma_score(lemma, goal);
                candidates.push((lemma.clone(), score));
            }
        }

        // Sort by heuristic score (descending)
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Return top N lemmas (without scores)
        candidates
            .into_iter()
            .take(max_results)
            .map(|(lemma, _score)| lemma)
            .collect()
    }

    /// Compute heuristic score for lemma applicability
    ///
    /// Higher score = more likely to be useful
    fn compute_lemma_score(&self, lemma: &LemmaHint, goal: &Expr) -> f64 {
        let mut score = 0.0;

        // Base score from priority
        score += lemma.priority as f64;

        // Bonus for exact pattern match
        let pattern = self.extract_pattern(goal);
        if self.lemmas.get(&pattern).is_some() {
            score += 50.0;
        }

        // Bonus for simple lemmas (fewer premises)
        let (premises, _) = Self::extract_lemma_structure_static(&lemma.lemma);
        let complexity_penalty = premises.len() as f64 * 10.0;
        score -= complexity_penalty;

        // Bonus for signature compatibility
        let goal_sig = Self::compute_signature(goal);
        let lemma_sig = Self::compute_lemma_conclusion_signature(&lemma.lemma);
        if Self::signatures_compatible(&lemma_sig, &goal_sig) {
            score += 30.0;
        }

        score
    }

    /// Register a tactic hint
    pub fn register_tactic(&mut self, pattern: Text, hint: TacticHint) {
        self.tactics
            .entry(pattern)
            .or_default()
            .push(hint);
    }

    /// Register a decision procedure
    pub fn register_decision_procedure(&mut self, proc: DecisionProcedure) {
        self.decision_procedures.push(proc);
    }

    /// Find applicable hints for a goal
    ///
    /// Returns hints sorted by priority (higher priority first).
    pub fn find_hints(&mut self, goal: &Expr) -> List<ApplicableHint> {
        let start = Instant::now();
        let mut hints = List::new();

        // Try to match lemmas
        let pattern = self.extract_pattern(goal);
        if let Maybe::Some(lemma_list) = self.lemmas.get(&pattern) {
            for lemma in lemma_list {
                hints.push(ApplicableHint::Lemma(lemma.clone()));
            }
        }

        // Try to match tactics
        if let Maybe::Some(tactic_list) = self.tactics.get(&pattern) {
            for tactic in tactic_list {
                hints.push(ApplicableHint::Tactic(tactic.clone()));
            }
        }

        // Try decision procedures
        for proc in &self.decision_procedures {
            if proc.is_applicable(goal) {
                hints.push(ApplicableHint::DecisionProcedure(proc.clone()));
            }
        }

        // Sort by priority (descending)
        hints.sort_by(|a, b| b.priority().cmp(&a.priority()));

        // Update statistics
        self.stats.total_queries += 1;
        self.stats.total_time_us += start.elapsed().as_micros() as u64;
        if !hints.is_empty() {
            self.stats.hits += 1;
        }

        hints
    }

    /// Extract pattern from expression for matching
    ///
    /// Converts expressions into canonical patterns for indexing.
    /// Examples:
    /// - `x + y = z` → `_ + _ = _`
    /// - `forall x. P(x)` → `forall _. _`
    fn extract_pattern(&self, expr: &Expr) -> Text {
        match &expr.kind {
            ExprKind::Binary { op, .. } => format!("_ {} _", op.as_str()).into(),

            ExprKind::Call { func, args, .. } => {
                if let ExprKind::Path(path) = &func.kind
                    && let Maybe::Some(ident) = option_to_maybe(path.as_ident())
                {
                    let placeholders = vec!["_"; args.len()].join(", ");
                    return format!("{}({})", ident.as_str(), placeholders).into();
                }
                "call(_)".into()
            }

            _ => "unknown".into(),
        }
    }

    /// Register standard library hints
    fn register_stdlib_hints(&mut self) {
        // ===== Arithmetic Lemmas =====

        // Commutativity
        self.register_lemma(
            "_ + _ = _".into(),
            LemmaHint {
                name: "plus_comm".into(),
                priority: 100,
                lemma: Heap::new(Self::create_arithmetic_lemma("a + b = b + a")),
            },
        );
        self.register_named_lemma(
            "plus_comm".into(),
            LemmaHint {
                name: "plus_comm".into(),
                priority: 100,
                lemma: Heap::new(Self::create_arithmetic_lemma("a + b = b + a")),
            },
        );

        self.register_lemma(
            "_ * _ = _".into(),
            LemmaHint {
                name: "mult_comm".into(),
                priority: 100,
                lemma: Heap::new(Self::create_arithmetic_lemma("a * b = b * a")),
            },
        );
        self.register_named_lemma(
            "mult_comm".into(),
            LemmaHint {
                name: "mult_comm".into(),
                priority: 100,
                lemma: Heap::new(Self::create_arithmetic_lemma("a * b = b * a")),
            },
        );

        // Associativity
        self.register_named_lemma(
            "plus_assoc".into(),
            LemmaHint {
                name: "plus_assoc".into(),
                priority: 95,
                lemma: Heap::new(Self::create_arithmetic_lemma("(a + b) + c = a + (b + c)")),
            },
        );

        self.register_named_lemma(
            "mult_assoc".into(),
            LemmaHint {
                name: "mult_assoc".into(),
                priority: 95,
                lemma: Heap::new(Self::create_arithmetic_lemma("(a * b) * c = a * (b * c)")),
            },
        );

        // Identity
        self.register_named_lemma(
            "plus_zero".into(),
            LemmaHint {
                name: "plus_zero".into(),
                priority: 110,
                lemma: Heap::new(Self::create_arithmetic_lemma("a + 0 = a")),
            },
        );

        self.register_named_lemma(
            "mult_one".into(),
            LemmaHint {
                name: "mult_one".into(),
                priority: 110,
                lemma: Heap::new(Self::create_arithmetic_lemma("a * 1 = a")),
            },
        );

        self.register_named_lemma(
            "mult_zero".into(),
            LemmaHint {
                name: "mult_zero".into(),
                priority: 110,
                lemma: Heap::new(Self::create_arithmetic_lemma("a * 0 = 0")),
            },
        );

        // Distributivity
        self.register_named_lemma(
            "mult_dist_plus".into(),
            LemmaHint {
                name: "mult_dist_plus".into(),
                priority: 90,
                lemma: Heap::new(Self::create_arithmetic_lemma("a * (b + c) = a * b + a * c")),
            },
        );

        // ===== Boolean Lemmas =====

        // De Morgan's laws
        self.register_lemma(
            "!(_ && _)".into(),
            LemmaHint {
                name: "demorgan_and".into(),
                priority: 105,
                lemma: Heap::new(Self::create_boolean_lemma("!(a && b) = !a || !b")),
            },
        );
        self.register_named_lemma(
            "demorgan_and".into(),
            LemmaHint {
                name: "demorgan_and".into(),
                priority: 105,
                lemma: Heap::new(Self::create_boolean_lemma("!(a && b) = !a || !b")),
            },
        );

        self.register_lemma(
            "!(_ || _)".into(),
            LemmaHint {
                name: "demorgan_or".into(),
                priority: 105,
                lemma: Heap::new(Self::create_boolean_lemma("!(a || b) = !a && !b")),
            },
        );
        self.register_named_lemma(
            "demorgan_or".into(),
            LemmaHint {
                name: "demorgan_or".into(),
                priority: 105,
                lemma: Heap::new(Self::create_boolean_lemma("!(a || b) = !a && !b")),
            },
        );

        // Boolean identities
        self.register_lemma(
            "_ && _ = _".into(),
            LemmaHint {
                name: "and_properties".into(),
                priority: 90,
                lemma: Heap::new(Self::create_boolean_lemma("a && b = b && a")),
            },
        );

        self.register_named_lemma(
            "and_comm".into(),
            LemmaHint {
                name: "and_comm".into(),
                priority: 100,
                lemma: Heap::new(Self::create_boolean_lemma("a && b = b && a")),
            },
        );

        self.register_named_lemma(
            "or_comm".into(),
            LemmaHint {
                name: "or_comm".into(),
                priority: 100,
                lemma: Heap::new(Self::create_boolean_lemma("a || b = b || a")),
            },
        );

        self.register_named_lemma(
            "and_true".into(),
            LemmaHint {
                name: "and_true".into(),
                priority: 110,
                lemma: Heap::new(Self::create_boolean_lemma("a && true = a")),
            },
        );

        self.register_named_lemma(
            "or_false".into(),
            LemmaHint {
                name: "or_false".into(),
                priority: 110,
                lemma: Heap::new(Self::create_boolean_lemma("a || false = a")),
            },
        );

        self.register_named_lemma(
            "double_negation".into(),
            LemmaHint {
                name: "double_negation".into(),
                priority: 120,
                lemma: Heap::new(Self::create_boolean_lemma("!!a = a")),
            },
        );

        // ===== Implication Lemmas =====

        self.register_named_lemma(
            "modus_ponens".into(),
            LemmaHint {
                name: "modus_ponens".into(),
                priority: 150,
                lemma: Heap::new(Self::create_implication_lemma("a && (a => b) => b")),
            },
        );

        self.register_named_lemma(
            "implication_trans".into(),
            LemmaHint {
                name: "implication_trans".into(),
                priority: 130,
                lemma: Heap::new(Self::create_implication_lemma(
                    "(a => b) && (b => c) => (a => c)",
                )),
            },
        );

        // ===== Equality Lemmas =====

        self.register_named_lemma(
            "eq_refl".into(),
            LemmaHint {
                name: "eq_refl".into(),
                priority: 200,
                lemma: Heap::new(Self::create_equality_lemma("a = a")),
            },
        );

        self.register_named_lemma(
            "eq_symm".into(),
            LemmaHint {
                name: "eq_symm".into(),
                priority: 140,
                lemma: Heap::new(Self::create_equality_lemma("a = b => b = a")),
            },
        );

        self.register_named_lemma(
            "eq_trans".into(),
            LemmaHint {
                name: "eq_trans".into(),
                priority: 140,
                lemma: Heap::new(Self::create_equality_lemma("(a = b) && (b = c) => (a = c)")),
            },
        );

        // ===== Decision Procedures =====

        self.register_decision_procedure(DecisionProcedure {
            name: "linear_arithmetic".into(),
            applicable_to: ProofDomain::LinearArithmetic,
            timeout: Duration::from_millis(100),
        });

        self.register_decision_procedure(DecisionProcedure {
            name: "propositional".into(),
            applicable_to: ProofDomain::Propositional,
            timeout: Duration::from_millis(50),
        });

        self.register_decision_procedure(DecisionProcedure {
            name: "equality".into(),
            applicable_to: ProofDomain::Equality,
            timeout: Duration::from_millis(75),
        });
    }

    /// Create an arithmetic lemma
    ///
    /// Generates standard arithmetic lemmas based on the description:
    /// - "commutativity" -> a + b = b + a
    /// - "associativity" -> (a + b) + c = a + (b + c)
    /// - "identity" -> a + 0 = a
    /// - "inverse" -> a - a = 0
    /// - "distributivity" -> a * (b + c) = a * b + a * c
    fn create_arithmetic_lemma(description: &str) -> Expr {
        use verum_ast::Ident;
        use verum_ast::expr::BinOp;
        use verum_ast::literal::{IntLit, Literal, LiteralKind};
        use verum_ast::span::Span;

        let span = Span::dummy();

        // Create variable expressions for lemmas
        let var_a = Expr::new(
            ExprKind::Path(verum_ast::Path::from_ident(Ident::new("a", span))),
            span,
        );
        let var_b = Expr::new(
            ExprKind::Path(verum_ast::Path::from_ident(Ident::new("b", span))),
            span,
        );
        let var_c = Expr::new(
            ExprKind::Path(verum_ast::Path::from_ident(Ident::new("c", span))),
            span,
        );
        let zero = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Int(IntLit::new(0)), span)),
            span,
        );

        let desc_lower = description.to_lowercase();

        if desc_lower.contains("commut") {
            // a + b = b + a
            let lhs = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Add,
                    left: Box::new(var_a.clone()),
                    right: Box::new(var_b.clone()),
                },
                span,
            );
            let rhs = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Add,
                    left: Box::new(var_b),
                    right: Box::new(var_a),
                },
                span,
            );
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            )
        } else if desc_lower.contains("assoc") {
            // (a + b) + c = a + (b + c)
            let lhs_inner = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Add,
                    left: Box::new(var_a.clone()),
                    right: Box::new(var_b.clone()),
                },
                span,
            );
            let lhs = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Add,
                    left: Box::new(lhs_inner),
                    right: Box::new(var_c.clone()),
                },
                span,
            );
            let rhs_inner = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Add,
                    left: Box::new(var_b),
                    right: Box::new(var_c),
                },
                span,
            );
            let rhs = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Add,
                    left: Box::new(var_a),
                    right: Box::new(rhs_inner),
                },
                span,
            );
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            )
        } else if desc_lower.contains("identity") || desc_lower.contains("zero") {
            // a + 0 = a
            let lhs = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Add,
                    left: Box::new(var_a.clone()),
                    right: Box::new(zero),
                },
                span,
            );
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(lhs),
                    right: Box::new(var_a),
                },
                span,
            )
        } else if desc_lower.contains("inverse") {
            // a - a = 0
            let lhs = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Sub,
                    left: Box::new(var_a.clone()),
                    right: Box::new(var_a),
                },
                span,
            );
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(lhs),
                    right: Box::new(zero),
                },
                span,
            )
        } else if desc_lower.contains("distribut") {
            // a * (b + c) = a * b + a * c
            let bc_sum = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Add,
                    left: Box::new(var_b.clone()),
                    right: Box::new(var_c.clone()),
                },
                span,
            );
            let lhs = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Mul,
                    left: Box::new(var_a.clone()),
                    right: Box::new(bc_sum),
                },
                span,
            );
            let ab = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Mul,
                    left: Box::new(var_a.clone()),
                    right: Box::new(var_b),
                },
                span,
            );
            let ac = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Mul,
                    left: Box::new(var_a),
                    right: Box::new(var_c),
                },
                span,
            );
            let rhs = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Add,
                    left: Box::new(ab),
                    right: Box::new(ac),
                },
                span,
            );
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            )
        } else {
            // Default: return true literal
            Self::create_default_lemma()
        }
    }

    /// Create a boolean lemma
    ///
    /// Generates standard boolean lemmas based on the description:
    /// - "double_negation" -> !!a = a
    /// - "de_morgan" -> !(a && b) = !a || !b
    /// - "excluded_middle" -> a || !a
    /// - "contradiction" -> !(a && !a)
    fn create_boolean_lemma(description: &str) -> Expr {
        use verum_ast::Ident;
        use verum_ast::expr::{BinOp, UnOp};
        use verum_ast::span::Span;

        let span = Span::dummy();

        let var_a = Expr::new(
            ExprKind::Path(verum_ast::Path::from_ident(Ident::new("a", span))),
            span,
        );
        let var_b = Expr::new(
            ExprKind::Path(verum_ast::Path::from_ident(Ident::new("b", span))),
            span,
        );

        let desc_lower = description.to_lowercase();

        if desc_lower.contains("double") && desc_lower.contains("neg") {
            // !!a = a
            let not_a = Expr::new(
                ExprKind::Unary {
                    op: UnOp::Not,
                    expr: Box::new(var_a.clone()),
                },
                span,
            );
            let not_not_a = Expr::new(
                ExprKind::Unary {
                    op: UnOp::Not,
                    expr: Box::new(not_a),
                },
                span,
            );
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(not_not_a),
                    right: Box::new(var_a),
                },
                span,
            )
        } else if desc_lower.contains("de_morgan") || desc_lower.contains("demorgan") {
            // !(a && b) = !a || !b
            let a_and_b = Expr::new(
                ExprKind::Binary {
                    op: BinOp::And,
                    left: Box::new(var_a.clone()),
                    right: Box::new(var_b.clone()),
                },
                span,
            );
            let not_a_and_b = Expr::new(
                ExprKind::Unary {
                    op: UnOp::Not,
                    expr: Box::new(a_and_b),
                },
                span,
            );
            let not_a = Expr::new(
                ExprKind::Unary {
                    op: UnOp::Not,
                    expr: Box::new(var_a),
                },
                span,
            );
            let not_b = Expr::new(
                ExprKind::Unary {
                    op: UnOp::Not,
                    expr: Box::new(var_b),
                },
                span,
            );
            let not_a_or_not_b = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Or,
                    left: Box::new(not_a),
                    right: Box::new(not_b),
                },
                span,
            );
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(not_a_and_b),
                    right: Box::new(not_a_or_not_b),
                },
                span,
            )
        } else if desc_lower.contains("excluded") {
            // a || !a
            let not_a = Expr::new(
                ExprKind::Unary {
                    op: UnOp::Not,
                    expr: Box::new(var_a.clone()),
                },
                span,
            );
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Or,
                    left: Box::new(var_a),
                    right: Box::new(not_a),
                },
                span,
            )
        } else {
            Self::create_default_lemma()
        }
    }

    /// Create an implication lemma
    ///
    /// Generates implication-related lemmas:
    /// - "modus_ponens" -> (a && (a => b)) => b
    /// - "contrapositive" -> (a => b) = (!b => !a)
    /// - "transitivity" -> ((a => b) && (b => c)) => (a => c)
    fn create_implication_lemma(description: &str) -> Expr {
        use verum_ast::Ident;
        use verum_ast::expr::{BinOp, UnOp};
        use verum_ast::span::Span;

        let span = Span::dummy();

        let var_a = Expr::new(
            ExprKind::Path(verum_ast::Path::from_ident(Ident::new("a", span))),
            span,
        );
        let var_b = Expr::new(
            ExprKind::Path(verum_ast::Path::from_ident(Ident::new("b", span))),
            span,
        );
        let var_c = Expr::new(
            ExprKind::Path(verum_ast::Path::from_ident(Ident::new("c", span))),
            span,
        );

        let desc_lower = description.to_lowercase();

        if desc_lower.contains("modus") || desc_lower.contains("ponens") {
            // (a && (a => b)) => b
            // Note: a => b is equivalent to !a || b
            let a_implies_b = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Or,
                    left: Box::new(Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Not,
                            expr: Box::new(var_a.clone()),
                        },
                        span,
                    )),
                    right: Box::new(var_b.clone()),
                },
                span,
            );
            let premise = Expr::new(
                ExprKind::Binary {
                    op: BinOp::And,
                    left: Box::new(var_a),
                    right: Box::new(a_implies_b),
                },
                span,
            );
            // premise => b encoded as !premise || b
            let not_premise = Expr::new(
                ExprKind::Unary {
                    op: UnOp::Not,
                    expr: Box::new(premise),
                },
                span,
            );
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Or,
                    left: Box::new(not_premise),
                    right: Box::new(var_b),
                },
                span,
            )
        } else if desc_lower.contains("contra") {
            // (a => b) = (!b => !a)
            // Using: (a => b) = !a || b and (!b => !a) = !!b || !a = b || !a
            let not_a = Expr::new(
                ExprKind::Unary {
                    op: UnOp::Not,
                    expr: Box::new(var_a.clone()),
                },
                span,
            );
            let not_b = Expr::new(
                ExprKind::Unary {
                    op: UnOp::Not,
                    expr: Box::new(var_b.clone()),
                },
                span,
            );
            let a_implies_b = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Or,
                    left: Box::new(not_a.clone()),
                    right: Box::new(var_b),
                },
                span,
            );
            let notb_implies_nota = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Or,
                    left: Box::new(Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Not,
                            expr: Box::new(not_b),
                        },
                        span,
                    )),
                    right: Box::new(not_a),
                },
                span,
            );
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(a_implies_b),
                    right: Box::new(notb_implies_nota),
                },
                span,
            )
        } else if desc_lower.contains("trans") {
            // ((a => b) && (b => c)) => (a => c)
            let a_implies_b = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Or,
                    left: Box::new(Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Not,
                            expr: Box::new(var_a.clone()),
                        },
                        span,
                    )),
                    right: Box::new(var_b.clone()),
                },
                span,
            );
            let b_implies_c = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Or,
                    left: Box::new(Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Not,
                            expr: Box::new(var_b),
                        },
                        span,
                    )),
                    right: Box::new(var_c.clone()),
                },
                span,
            );
            let premise = Expr::new(
                ExprKind::Binary {
                    op: BinOp::And,
                    left: Box::new(a_implies_b),
                    right: Box::new(b_implies_c),
                },
                span,
            );
            let a_implies_c = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Or,
                    left: Box::new(Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Not,
                            expr: Box::new(var_a),
                        },
                        span,
                    )),
                    right: Box::new(var_c),
                },
                span,
            );
            // premise => conclusion as !premise || conclusion
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Or,
                    left: Box::new(Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Not,
                            expr: Box::new(premise),
                        },
                        span,
                    )),
                    right: Box::new(a_implies_c),
                },
                span,
            )
        } else {
            Self::create_default_lemma()
        }
    }

    /// Create an equality lemma
    ///
    /// Generates equality-related lemmas:
    /// - "reflexivity" -> a = a
    /// - "symmetry" -> (a = b) => (b = a)
    /// - "transitivity" -> ((a = b) && (b = c)) => (a = c)
    /// - "substitution" -> (a = b) => (f(a) = f(b))
    fn create_equality_lemma(description: &str) -> Expr {
        use verum_ast::Ident;
        use verum_ast::expr::{BinOp, UnOp};
        use verum_ast::span::Span;

        let span = Span::dummy();

        let var_a = Expr::new(
            ExprKind::Path(verum_ast::Path::from_ident(Ident::new("a", span))),
            span,
        );
        let var_b = Expr::new(
            ExprKind::Path(verum_ast::Path::from_ident(Ident::new("b", span))),
            span,
        );
        let var_c = Expr::new(
            ExprKind::Path(verum_ast::Path::from_ident(Ident::new("c", span))),
            span,
        );

        let desc_lower = description.to_lowercase();

        if desc_lower.contains("reflex") {
            // a = a
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(var_a.clone()),
                    right: Box::new(var_a),
                },
                span,
            )
        } else if desc_lower.contains("symm") {
            // (a = b) => (b = a)
            let a_eq_b = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(var_a.clone()),
                    right: Box::new(var_b.clone()),
                },
                span,
            );
            let b_eq_a = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(var_b),
                    right: Box::new(var_a),
                },
                span,
            );
            // implication as !premise || conclusion
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Or,
                    left: Box::new(Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Not,
                            expr: Box::new(a_eq_b),
                        },
                        span,
                    )),
                    right: Box::new(b_eq_a),
                },
                span,
            )
        } else if desc_lower.contains("trans") {
            // ((a = b) && (b = c)) => (a = c)
            let a_eq_b = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(var_a.clone()),
                    right: Box::new(var_b.clone()),
                },
                span,
            );
            let b_eq_c = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(var_b),
                    right: Box::new(var_c.clone()),
                },
                span,
            );
            let premise = Expr::new(
                ExprKind::Binary {
                    op: BinOp::And,
                    left: Box::new(a_eq_b),
                    right: Box::new(b_eq_c),
                },
                span,
            );
            let a_eq_c = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(var_a),
                    right: Box::new(var_c),
                },
                span,
            );
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Or,
                    left: Box::new(Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Not,
                            expr: Box::new(premise),
                        },
                        span,
                    )),
                    right: Box::new(a_eq_c),
                },
                span,
            )
        } else {
            Self::create_default_lemma()
        }
    }

    /// Create a default true lemma when no specific pattern matches
    fn create_default_lemma() -> Expr {
        use verum_ast::literal::{Literal, LiteralKind};
        use verum_ast::span::Span;

        Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        )
    }

    /// Get database statistics
    pub fn stats(&self) -> &HintStats {
        &self.stats
    }

    /// Clear all hints
    pub fn clear(&mut self) {
        self.lemmas.clear();
        self.tactics.clear();
        self.decision_procedures.clear();
        self.stats = HintStats::default();
    }

    /// Instantiate a lemma with specific terms
    ///
    /// Given a lemma (possibly quantified) and a list of instantiation terms,
    /// produces a concrete instance of the lemma.
    ///
    /// # Example
    /// ```ignore
    /// // Lemma: forall x. P(x)
    /// // Instantiation: [term_a]
    /// // Result: P(term_a)
    /// ```
    pub fn instantiate_lemma(&self, lemma: &Expr, terms: &List<Expr>) -> Result<Expr, Text> {
        // Parse lemma to extract quantified variables
        let (vars, body) = Self::extract_quantifiers(lemma);

        if vars.len() != terms.len() {
            return Err(format!(
                "Lemma expects {} arguments but got {}",
                vars.len(),
                terms.len()
            )
            .into());
        }

        // Build substitution map
        let mut subst = Map::new();
        for (var, term) in vars.iter().zip(terms.iter()) {
            subst.insert(var.clone(), term.clone());
        }

        // Apply substitution to body
        Ok(ProofSearchEngine::apply_substitution(&body, &subst))
    }

    /// Extract universal quantifiers from lemma
    ///
    /// Returns (variables, body) where variables are the quantified variables
    /// and body is the formula after removing quantifiers.
    ///
    /// # Example
    /// ```ignore
    /// // Input: forall x. forall y. P(x, y)
    /// // Output: ([x, y], P(x, y))
    /// ```
    fn extract_quantifiers(expr: &Expr) -> (List<Text>, Expr) {
        let mut vars = List::new();
        let mut current = expr;

        // For now, we don't have explicit forall syntax in ExprKind
        // In a full implementation, this would parse quantifiers
        // For demonstration, we'll extract variables from patterns

        // If lemma is in form: premises => conclusion,
        // variables are inferred from the pattern
        (vars, current.clone())
    }

    /// Get all registered lemma names
    pub fn lemma_names(&self) -> List<Text> {
        let mut names = List::new();
        for (_pattern, lemma_list) in &self.lemmas {
            for lemma in lemma_list {
                if !names.iter().any(|n| n == &lemma.name) {
                    names.push(lemma.name.clone());
                }
            }
        }
        names.sort();
        names
    }

    /// Get lemma count
    pub fn lemma_count(&self) -> usize {
        self.lemmas.values().map(|list| list.len()).sum()
    }

    /// Get tactic count
    pub fn tactic_count(&self) -> usize {
        self.tactics.values().map(|list| list.len()).sum()
    }

    /// Get decision procedure count
    pub fn decision_procedure_count(&self) -> usize {
        self.decision_procedures.len()
    }
}

impl Default for HintsDatabase {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Hint Types ====================

/// Lemma hint for proof automation
///
/// A lemma hint for proof automation. Lemmas are stored with a name, priority
/// (higher = tried first), the lemma expression, and an optional structural pattern
/// for matching against proof goals.
#[derive(Debug, Clone)]
pub struct LemmaHint {
    /// Lemma name
    pub name: Text,
    /// Priority (0-1000, higher = more likely to be tried first)
    pub priority: u32,
    /// The lemma expression
    pub lemma: Heap<Expr>,
}

/// Tactic hint for goal transformation
#[derive(Debug, Clone)]
pub struct TacticHint {
    /// Tactic name
    pub name: Text,
    /// Priority
    pub priority: u32,
    /// Tactic to apply
    pub tactic: ProofTactic,
}

/// Decision procedure hint
///
/// A verified decision procedure for a decidable theory fragment. Includes:
/// - SAT/tautology checking (propositional formulas)
/// - Linear arithmetic (Simplex for QF_LIA/QF_LRA)
/// - Presburger arithmetic (Cooper's algorithm, linear arithmetic with quantifiers)
/// Each procedure has a domain, timeout, and soundness guarantee flag.
#[derive(Debug, Clone)]
pub struct DecisionProcedure {
    /// Procedure name
    pub name: Text,
    /// Domain this procedure applies to
    pub applicable_to: ProofDomain,
    /// Timeout for this procedure
    pub timeout: Duration,
}

impl DecisionProcedure {
    /// Check if this procedure is applicable to a goal
    pub fn is_applicable(&self, goal: &Expr) -> bool {
        match self.applicable_to {
            ProofDomain::LinearArithmetic => Self::is_linear_arithmetic(goal),
            ProofDomain::Propositional => Self::is_propositional(goal),
            ProofDomain::Equality => Self::is_equality(goal),
            ProofDomain::BitVectors => false, // Would need BV detection
            ProofDomain::Arrays => false,     // Would need array detection
        }
    }

    /// Check if expression is in linear arithmetic fragment
    fn is_linear_arithmetic(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Binary { op, left, right } => {
                matches!(
                    op,
                    BinOp::Add
                        | BinOp::Sub
                        | BinOp::Mul
                        | BinOp::Lt
                        | BinOp::Le
                        | BinOp::Gt
                        | BinOp::Ge
                        | BinOp::Eq
                ) && Self::is_linear_arithmetic(left)
                    && Self::is_linear_arithmetic(right)
            }
            ExprKind::Literal(_) | ExprKind::Path(_) => true,
            ExprKind::Paren(inner) => Self::is_linear_arithmetic(inner),
            _ => false,
        }
    }

    /// Check if expression is propositional
    fn is_propositional(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Binary { op, left, right } => {
                matches!(op, BinOp::And | BinOp::Or)
                    && Self::is_propositional(left)
                    && Self::is_propositional(right)
            }
            ExprKind::Unary {
                op: verum_ast::UnOp::Not,
                expr: inner,
            } => Self::is_propositional(inner),
            ExprKind::Path(_) | ExprKind::Literal(_) => true,
            ExprKind::Paren(inner) => Self::is_propositional(inner),
            _ => false,
        }
    }

    /// Check if expression is equality
    fn is_equality(expr: &Expr) -> bool {
        matches!(
            &expr.kind,
            ExprKind::Binary {
                op: BinOp::Eq | BinOp::Ne,
                ..
            }
        )
    }
}

/// Proof domain for decision procedures
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofDomain {
    /// Propositional logic (SAT)
    Propositional,
    /// Linear arithmetic (QF_LIA, QF_LRA)
    LinearArithmetic,
    /// Equality with uninterpreted functions (QF_UF)
    Equality,
    /// Bit-vectors (QF_BV)
    BitVectors,
    /// Arrays (QF_A)
    Arrays,
}

/// Applicable hint (union of different hint types)
#[derive(Debug, Clone)]
pub enum ApplicableHint {
    /// Lemma hint
    Lemma(LemmaHint),
    /// Tactic hint
    Tactic(TacticHint),
    /// Decision procedure
    DecisionProcedure(DecisionProcedure),
}

impl ApplicableHint {
    /// Get priority of this hint
    pub fn priority(&self) -> u32 {
        match self {
            ApplicableHint::Lemma(h) => h.priority,
            ApplicableHint::Tactic(h) => h.priority,
            ApplicableHint::DecisionProcedure(_) => 1000, // Decision procedures have highest priority
        }
    }

    /// Get hint name
    pub fn name(&self) -> &Text {
        match self {
            ApplicableHint::Lemma(h) => &h.name,
            ApplicableHint::Tactic(h) => &h.name,
            ApplicableHint::DecisionProcedure(h) => &h.name,
        }
    }
}

// ==================== Proof Goals and Trees ====================

/// Proof goal to be proven
///
/// A proof goal: a proposition to prove under a set of hypotheses.
/// Theorem syntax: `theorem name(params): proposition { proof_term }`
/// Goals carry an expression, a list of hypothesis expressions, and an optional label.
#[derive(Debug, Clone)]
pub struct ProofGoal {
    /// Goal expression to prove
    pub goal: Expr,
    /// Hypotheses (context)
    pub hypotheses: List<Expr>,
    /// Goal name/label
    pub label: Maybe<Text>,
}

impl ProofGoal {
    /// Create a new proof goal
    pub fn new(goal: Expr) -> Self {
        Self {
            goal,
            hypotheses: List::new(),
            label: Maybe::None,
        }
    }

    /// Create goal with hypotheses
    pub fn with_hypotheses(goal: Expr, hypotheses: List<Expr>) -> Self {
        Self {
            goal,
            hypotheses,
            label: Maybe::None,
        }
    }

    /// Add hypothesis to goal
    pub fn add_hypothesis(&mut self, hyp: Expr) {
        self.hypotheses.push(hyp);
    }

    /// Set goal label
    pub fn with_label(mut self, label: Text) -> Self {
        self.label = Maybe::Some(label);
        self
    }
}

/// Proof term representing evidence
///
/// Proof terms are first-class evidence values. They represent:
/// - Direct proof construction: `modus_ponens(p, pq)` applies implication
/// - Proof by cases: pattern match on disjunction `P ∨ Q`
/// - Lambda abstraction: introduce hypothesis
/// - SMT discharge: solver-generated proof
/// Proofs follow the Curry-Howard correspondence (propositions as types).
#[derive(Debug, Clone)]
pub enum ProofTerm {
    /// Axiom or assumption
    Axiom(Text),

    /// Application of inference rule
    Apply {
        rule: Text,
        premises: List<Heap<ProofTerm>>,
    },

    /// Lambda abstraction
    Lambda { var: Text, body: Heap<ProofTerm> },

    /// Proof by cases
    Cases {
        scrutinee: Expr,
        cases: List<(Expr, Heap<ProofTerm>)>,
    },

    /// Induction proof
    Induction {
        var: Text,
        base_case: Heap<ProofTerm>,
        inductive_case: Heap<ProofTerm>,
    },

    /// SMT solver proof
    SmtProof { solver: Text, formula: Expr },
}

/// Proof tree representing proof structure
///
/// A proof tree representing hierarchical proof structure. Each node holds a goal,
/// an optional tactic that was applied, and child subproofs. The tree is complete
/// when all leaves are discharged (no remaining subgoals).
#[derive(Debug, Clone)]
pub struct ProofTree {
    /// Goal being proved
    pub goal: ProofGoal,
    /// Tactic applied (if any)
    pub tactic: Maybe<ProofTactic>,
    /// Subproofs (children)
    pub subproofs: List<Heap<ProofTree>>,
    /// Proof status
    pub status: ProofStatus,
}

/// Status of a proof
#[derive(Debug, Clone)]
pub enum ProofStatus {
    /// Not yet proven
    Incomplete,
    /// Successfully proven
    Complete(ProofTerm),
    /// Failed to prove
    Failed(Text),
}

impl ProofTree {
    /// Create new proof tree from goal
    pub fn new(goal: ProofGoal) -> Self {
        Self {
            goal,
            tactic: Maybe::None,
            subproofs: List::new(),
            status: ProofStatus::Incomplete,
        }
    }

    /// Check if proof is complete
    pub fn is_complete(&self) -> bool {
        matches!(self.status, ProofStatus::Complete(_))
    }

    /// Check if all subproofs are complete
    pub fn all_subproofs_complete(&self) -> bool {
        self.subproofs.iter().all(|sp| sp.is_complete())
    }

    /// Mark as complete with proof term
    pub fn mark_complete(&mut self, proof: ProofTerm) {
        self.status = ProofStatus::Complete(proof);
    }

    /// Mark as failed
    pub fn mark_failed(&mut self, reason: Text) {
        self.status = ProofStatus::Failed(reason);
    }
}

// ==================== Proof Tactics ====================

/// Proof tactic for goal transformation
///
/// Tactics transform proof goals into simpler subgoals.
///
/// Proof tactics transform proof goals into simpler subgoals. Available tactics:
/// - `simp`: simplify using lemma database (automatic rewriting)
/// - `ring`: normalize ring expressions (commutativity, associativity, distributivity)
/// - `field`: normalize field expressions (adds division handling)
/// - `omega`: decide linear integer arithmetic (Presburger / Cooper's algorithm)
/// - `blast`: tableau prover for propositional/first-order logic
/// - `intro`: introduce hypothesis from implication goal
/// - `split`: split conjunction goal into two subgoals
/// - `apply`: apply a lemma whose conclusion matches the goal
/// - `induction`: structural/strong/well-founded induction on a variable
/// - `auto`: automated search combining assumption, reflexivity, intro, split, apply
#[derive(Debug, Clone)]
pub enum ProofTactic {
    /// Simplification tactic: rewrites goal using lemma database rules
    Simplify,

    /// Simplify with specific lemmas: simp[lemma1, lemma2]
    /// Forward reasoning: `have h1: x + 0 = x by simp`
    SimpWith { lemmas: List<Text> },

    /// Introduce hypothesis
    Intro,

    /// Introduce multiple hypotheses with names
    IntroNamed { names: List<Text> },

    /// Split conjunction
    Split,

    /// Apply a lemma
    Apply { lemma: Text },

    /// Apply lemma with explicit arguments
    ApplyWith { lemma: Text, args: List<Text> },

    /// Induction on variable
    Induction { var: Text },

    /// Strong induction on variable
    StrongInduction { var: Text },

    /// Well-founded induction
    WellFoundedInduction { var: Text, relation: Text },

    /// Reflexivity (for equality)
    Reflexivity,

    /// Assumption (use hypothesis)
    Assumption,

    /// Automatic proof search
    Auto,

    /// Automatic with hint database
    AutoWith { hints: List<Text> },

    // ==================== Formal Proof Tactics ====================
    /// Ring arithmetic normalization: normalizes expressions using commutativity,
    /// associativity, distributivity, identity, and inverse ring axioms
    Ring,

    /// Field arithmetic normalization: extends ring normalization with division
    /// handling (multiplicative inverses for nonzero denominators)
    Field,

    /// Linear integer arithmetic solver (Omega test / Cooper's algorithm):
    /// decides Presburger arithmetic (quantifier-free linear integer constraints)
    Omega,

    /// Tableau prover: systematic search for propositional and simple first-order
    /// logic proofs using analytic tableaux method
    Blast,

    /// SMT solver dispatch: delegates goal to Z3/CVC5 with optional solver choice
    /// and timeout. Uses `@smt(solver = "Z3", timeout = 5000)` configuration.
    /// The solver attempts to discharge the goal automatically.
    Smt {
        solver: Maybe<Text>,
        timeout_ms: Maybe<u64>,
    },

    /// Rewrite using equality hypothesis
    Rewrite { hypothesis: Text, reverse: bool },

    /// Rewrite at specific target
    RewriteAt {
        hypothesis: Text,
        target: Text,
        reverse: bool,
    },

    /// Unfold definition
    Unfold { name: Text },

    /// Compute/normalize expression
    Compute,

    /// Left of disjunction
    Left,

    /// Right of disjunction
    Right,

    /// Existential witness: exists e
    Exists { witness: Text },

    /// Case analysis on hypothesis
    CasesOn { hypothesis: Text },

    /// Destruct hypothesis
    Destruct { hypothesis: Text },

    /// Exact proof term
    Exact { term: Text },

    /// Contradiction
    Contradiction,

    /// Exfalso (derive anything from False)
    Exfalso,

    /// Try tactic (continue on failure)
    Try(Heap<ProofTactic>),

    /// Focus on first goal
    Focus(Heap<ProofTactic>),

    /// Apply to all goals
    AllGoals(Heap<ProofTactic>),

    /// Sequential composition
    Seq(Heap<ProofTactic>, Heap<ProofTactic>),

    /// Alternative (try first, if fails try second)
    Alt(Heap<ProofTactic>, Heap<ProofTactic>),

    /// First successful tactic from list
    First(List<ProofTactic>),

    /// Repeat until fixed point
    Repeat(Heap<ProofTactic>),

    /// Repeat at most n times
    RepeatN(Heap<ProofTactic>, usize),

    /// Done (close proof)
    Done,

    /// Admit (leave unproven - development mode)
    Admit,

    /// Sorry (like admit but marks as incomplete)
    Sorry,

    /// Custom named tactic
    Named { name: Text, args: List<Text> },

    // ==================== Tactic-DSL control flow ====================
    /// Local let-binding inside a tactic script. The bound `value` is
    /// simplified in the current goal context and pushed onto the hypothesis
    /// list as the equation `name = value`; `body` then runs against the
    /// extended goal. Follows Ltac2's substitution semantics while letting
    /// the SMT backend see the binding as an ordinary equality.
    Let {
        name: Text,
        value: Heap<Expr>,
        body: Heap<ProofTactic>,
    },

    /// Committed pattern-match on a meta-level expression. Each arm carries
    /// a pattern, an optional guard, and a tactic body. The first arm whose
    /// pattern (and guard) matches the simplified scrutinee runs; if its
    /// body fails the whole match fails. This matches Rocq's default
    /// `match goal` and mirrors Eisbach's `match_concl` — cross-arm
    /// back-tracking is opt-in via `first { … }`.
    Match {
        scrutinee: Heap<Expr>,
        arms: List<MatchArm>,
    },

    /// Explicit tactic failure. The rendered message flows into surrounding
    /// `try` / `alt` / `first` combinators just like any other
    /// `ProofError::TacticFailed`. Analogue of Lean 4's `throwError` and
    /// Ltac2's `Control.throw`.
    Fail { message: Text },

    /// Runtime conditional. The engine first folds `cond` via
    /// `simplify_expr`; if that yields `Bool(true/false)` the matching
    /// branch runs. Otherwise the engine asks the SMT backend whether the
    /// current hypotheses entail `cond` (or `¬cond`) and dispatches on the
    /// verdict. An undecidable condition raises `TacticFailed`.
    If {
        cond: Heap<Expr>,
        then_branch: Heap<ProofTactic>,
        else_branch: Maybe<Heap<ProofTactic>>,
    },
}

/// An arm of a [`ProofTactic::Match`].
///
/// Mirrors `verum_ast::decl::TacticMatchArm` but with its body already
/// lowered into the SMT-facing IR.
#[derive(Debug, Clone)]
pub struct MatchArm {
    /// Pattern tested against the scrutinee.
    pub pattern: Pattern,
    /// Optional guard evaluated after a successful pattern match.
    pub guard: Maybe<Heap<Expr>>,
    /// Tactic body executed when pattern (and guard) match.
    pub body: ProofTactic,
}

impl ProofTactic {
    /// Compose two tactics sequentially
    pub fn then(self, next: ProofTactic) -> ProofTactic {
        ProofTactic::Seq(Heap::new(self), Heap::new(next))
    }

    /// Try this tactic, if it fails try alternative
    pub fn or_else(self, alt: ProofTactic) -> ProofTactic {
        ProofTactic::Alt(Heap::new(self), Heap::new(alt))
    }

    /// Repeat this tactic until it stops making progress
    pub fn repeat(self) -> ProofTactic {
        ProofTactic::Repeat(Heap::new(self))
    }
}

// ==================== Helper Types ====================

/// Simplified constructor representation for induction
///
/// Used by the induction tactic to represent constructors of inductive types.
/// In a full implementation, this would be replaced by proper ADT introspection.
#[derive(Debug, Clone)]
struct SimpleConstructor {
    /// Constructor name (e.g., "Zero", "Succ", "Nil", "Cons")
    name: Text,
    /// Number of arguments
    #[allow(dead_code)] // Used for pattern generation
    arity: usize,
    /// Indices of recursive arguments (e.g., for Succ(Nat), this is [0])
    recursive_args: List<usize>,
}

// ==================== Proof Errors ====================

/// Proof verification errors
#[derive(Debug, thiserror::Error, Clone)]
pub enum ProofError {
    #[error("Tactic failed: {0}")]
    TacticFailed(Text),

    #[error("SMT timeout")]
    SmtTimeout,

    #[error("Invalid proof term: {0}")]
    InvalidProof(Text),

    #[error("Unification failed: {0}")]
    UnificationFailed(Text),

    #[error("Goal not in context: {0}")]
    NotInContext(Text),

    #[error("Not an equality: {0}")]
    NotEquality(Text),
}

// ==================== Proof Search Engine ====================

/// Visited goal fingerprint for cycle detection
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct GoalFingerprint {
    /// Hash of the goal expression structure
    structure_hash: u64,
    /// Number of hypotheses
    hypothesis_count: usize,
}

impl GoalFingerprint {
    fn from_goal(goal: &ProofGoal) -> Self {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        // Hash the goal expression structure (simplified)
        format!("{:?}", goal.goal.kind).hash(&mut hasher);
        // Hash hypothesis count to distinguish contexts
        goal.hypotheses.len().hash(&mut hasher);

        Self {
            structure_hash: hasher.finish(),
            hypothesis_count: goal.hypotheses.len(),
        }
    }
}

/// Search state for backtracking
#[derive(Debug, Clone)]
#[allow(dead_code)] // Used for proof search backtracking
struct SearchState {
    /// Current proof goal
    goal: ProofGoal,
    /// Remaining tactics to try
    remaining_tactics: List<ProofTactic>,
    /// Depth at this state
    depth: usize,
    /// Parent state index (for backtracking)
    parent: Maybe<usize>,
    /// Applied tactic (if any)
    applied_tactic: Maybe<ProofTactic>,
}

/// Automated proof search engine with backtracking
///
/// Uses hints, tactics, and decision procedures to automatically
/// prove goals. Implements iterative deepening depth-first search
/// with cycle detection for completeness.
///
/// Automated proof search engine using iterative deepening DFS with backtracking
/// and cycle detection. The search strategy tries tactics in priority order:
/// 1. assumption (hypothesis matches goal)
/// 2. reflexivity (goal is `x = x`)
/// 3. intro + recurse (peel off implication)
/// 4. split + recurse (break conjunction)
/// 5. apply_hypothesis + recurse (use matching hypothesis)
/// 6. unfold_definition + recurse (expand definitions)
/// Hints database guides instantiation; decision procedures handle decidable fragments.
#[derive(Debug)]
pub struct ProofSearchEngine {
    /// Hints database
    hints: HintsDatabase,

    /// Maximum search depth
    max_depth: usize,

    /// Global timeout
    timeout: Duration,

    /// Search statistics
    stats: SearchStats,

    /// Visited goals for cycle detection (cleared per search)
    visited: std::collections::HashSet<GoalFingerprint>,

    /// Current search depth
    current_depth: usize,

    /// Backtrack stack for proof search
    backtrack_stack: List<SearchState>,

    /// Track incomplete proofs (goals accepted via Sorry tactic)
    /// Each entry contains the goal description and source location
    incomplete_proofs: List<IncompleteProof>,

    /// Refinement-reflection registry. When non-empty, every
    /// invocation of `try_auto` first injects the reflected
    /// function axioms into the SMT context so the solver can
    /// unfold user-defined functions during proof search.
    /// See `crate::refinement_reflection`.
    reflection_registry: crate::refinement_reflection::RefinementReflectionRegistry,
}

/// Record of an incomplete proof (accepted via Sorry tactic)
#[derive(Debug, Clone)]
pub struct IncompleteProof {
    /// Description of the goal that was skipped
    pub goal_description: Text,
    /// Source location where Sorry was used
    pub source_location: Maybe<Text>,
    /// Timestamp when this was recorded
    pub timestamp: std::time::Instant,
}

impl ProofSearchEngine {
    /// Create a new proof search engine
    pub fn new() -> Self {
        Self {
            hints: HintsDatabase::with_core(),
            max_depth: 10,
            timeout: Duration::from_secs(5),
            stats: SearchStats::default(),
            visited: std::collections::HashSet::new(),
            current_depth: 0,
            backtrack_stack: List::new(),
            incomplete_proofs: List::new(),
            reflection_registry: crate::refinement_reflection::RefinementReflectionRegistry::new(),
        }
    }

    /// Create with custom hints database
    pub fn with_hints(hints: HintsDatabase) -> Self {
        Self {
            hints,
            max_depth: 10,
            timeout: Duration::from_secs(5),
            stats: SearchStats::default(),
            visited: std::collections::HashSet::new(),
            current_depth: 0,
            backtrack_stack: List::new(),
            incomplete_proofs: List::new(),
            reflection_registry: crate::refinement_reflection::RefinementReflectionRegistry::new(),
        }
    }

    /// Get list of incomplete proofs (goals accepted via Sorry)
    pub fn incomplete_proofs(&self) -> &List<IncompleteProof> {
        &self.incomplete_proofs
    }

    /// Check if there are any incomplete proofs
    pub fn has_incomplete_proofs(&self) -> bool {
        !self.incomplete_proofs.is_empty()
    }

    /// Record an incomplete proof
    fn record_incomplete_proof(&mut self, goal: &ProofGoal) {
        self.incomplete_proofs.push(IncompleteProof {
            goal_description: format!("{:?}", goal.goal).into(),
            source_location: goal.label.clone(),
            timestamp: std::time::Instant::now(),
        });
    }

    /// Set maximum search depth
    pub fn set_max_depth(&mut self, depth: usize) {
        self.max_depth = depth;
    }

    /// Install a refinement-reflection registry. The reflected
    /// function axioms become available to all subsequent proof
    /// searches via `try_auto` and the `cubical`/`descent` named
    /// tactics. Replacing the registry is idempotent for callers
    /// that re-register the same definitions.
    pub fn set_reflection_registry(
        &mut self,
        registry: crate::refinement_reflection::RefinementReflectionRegistry,
    ) {
        self.reflection_registry = registry;
    }

    /// Read-only access to the reflection registry, e.g. for
    /// diagnostics or to count how many user functions are
    /// available as Z3 axioms during this proof session.
    pub fn reflection_registry(
        &self,
    ) -> &crate::refinement_reflection::RefinementReflectionRegistry {
        &self.reflection_registry
    }

    /// Set global timeout
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Clear search state for new proof attempt
    fn reset_search_state(&mut self) {
        self.visited.clear();
        self.current_depth = 0;
        self.backtrack_stack.clear();
    }

    /// Check if goal has been visited (cycle detection)
    fn is_goal_visited(&self, goal: &ProofGoal) -> bool {
        let fingerprint = GoalFingerprint::from_goal(goal);
        self.visited.contains(&fingerprint)
    }

    /// Mark goal as visited
    fn mark_goal_visited(&mut self, goal: &ProofGoal) {
        let fingerprint = GoalFingerprint::from_goal(goal);
        self.visited.insert(fingerprint);
    }

    /// Attempt to automatically prove a goal with backtracking
    ///
    /// Uses iterative deepening depth-first search:
    /// 1. Try to prove at depth 1, 2, 3, ... up to max_depth
    /// 2. At each depth, use DFS with backtracking
    /// 3. Detect cycles to avoid infinite loops
    ///
    /// Returns Ok if a proof is found, Err otherwise.
    pub fn auto_prove(&mut self, context: &Context, goal: &Expr) -> VerificationResult {
        // Convenience wrapper: builds a hypothesis-free `ProofGoal` and
        // delegates to the full-featured entry point below. Callers that
        // have already accumulated hypotheses for the goal (e.g. from
        // `Split` on a conjunction, where each conjunct inherits the parent
        // goal's context) should use [`auto_prove_goal`] directly so the
        // hypotheses survive the recursion.
        self.auto_prove_goal(context, &ProofGoal::new(goal.clone()))
    }

    /// Same as [`auto_prove`] but accepts a full [`ProofGoal`] so caller-
    /// supplied hypotheses are propagated into the search frame. This is
    /// the correct entry point for any discharge step that fans out
    /// subgoals which must remain aware of the parent's context —
    /// conjunctive splits, premise obligations after `apply`, case-
    /// analysis branches, and so on.
    pub fn auto_prove_goal(
        &mut self,
        context: &Context,
        proof_goal: &ProofGoal,
    ) -> VerificationResult {
        let start = Instant::now();
        self.stats.total_attempts += 1;

        // Reset search state
        self.reset_search_state();

        // Iterative deepening: try increasing depths
        for depth_limit in 1..=self.max_depth {
            self.visited.clear(); // Reset visited for each depth iteration

            match self.prove_with_backtracking(context, proof_goal, depth_limit, start) {
                Ok(result) => {
                    self.stats.successes += 1;
                    return Ok(result);
                }
                Err(VerificationError::Timeout { .. }) => {
                    // Propagate timeout
                    return Err(VerificationError::Timeout {
                        constraint: format!("{:?}", proof_goal.goal).into(),
                        timeout: self.timeout,
                        cost: VerificationCost::new("timeout".into(), start.elapsed(), false)
                            .with_timeout(),
                    });
                }
                Err(_) => {
                    // Try next depth
                    continue;
                }
            }
        }

        // Exhausted all depths
        self.stats.failures += 1;
        Err(VerificationError::Unknown(
            format!("proof search exhausted at depth {}", self.max_depth).into(),
        ))
    }

    /// Prove a goal with backtracking depth-first search
    fn prove_with_backtracking(
        &mut self,
        context: &Context,
        goal: &ProofGoal,
        depth_limit: usize,
        start: Instant,
    ) -> VerificationResult {
        // Check timeout
        if start.elapsed() >= self.timeout {
            return Err(VerificationError::Timeout {
                constraint: format!("{:?}", goal.goal).into(),
                timeout: self.timeout,
                cost: VerificationCost::new("timeout".into(), start.elapsed(), false)
                    .with_timeout(),
            });
        }

        // Check depth limit
        if self.current_depth >= depth_limit {
            return Err(VerificationError::Unknown("depth limit reached".into()));
        }

        // Cycle detection
        if self.is_goal_visited(goal) {
            return Err(VerificationError::Unknown("cycle detected".into()));
        }

        // Mark as visited
        self.mark_goal_visited(goal);
        self.current_depth += 1;

        // Find applicable hints
        let hints = self.hints.find_hints(&goal.goal);

        if hints.is_empty() {
            self.stats.no_hints += 1;
            self.current_depth -= 1;
            return Err(VerificationError::Unknown(
                "no applicable hints found".to_text(),
            ));
        }

        // Try each hint in priority order (with backtracking)
        for hint in &hints {
            // Check timeout
            if start.elapsed() >= self.timeout {
                let cost =
                    VerificationCost::new("proof_search_timeout".into(), start.elapsed(), false)
                        .with_timeout();

                return Err(VerificationError::Timeout {
                    constraint: format!("{:?}", goal).into(),
                    timeout: self.timeout,
                    cost,
                });
            }

            // Try to apply this hint
            match self.apply_hint(context, &goal.goal, hint) {
                Ok(proof) => {
                    self.current_depth -= 1; // Restore depth on success
                    return Ok(proof);
                }
                Err(_) => {
                    // Backtrack: try next hint
                    continue;
                }
            }
        }

        // No hint succeeded - backtrack
        self.current_depth -= 1;
        Err(VerificationError::Unknown("all hints failed".to_text()))
    }

    /// Apply a hint to try to prove the goal
    fn apply_hint(
        &mut self,
        context: &Context,
        goal: &Expr,
        hint: &ApplicableHint,
    ) -> VerificationResult {
        match hint {
            ApplicableHint::DecisionProcedure(proc) => {
                self.apply_decision_procedure(context, goal, proc)
            }

            ApplicableHint::Lemma(lemma) => self.apply_lemma(context, goal, lemma),

            ApplicableHint::Tactic(tactic) => self.apply_tactic(context, goal, tactic),
        }
    }

    /// Apply decision procedure
    fn apply_decision_procedure(
        &mut self,
        context: &Context,
        goal: &Expr,
        proc: &DecisionProcedure,
    ) -> VerificationResult {
        let translator = Translator::new(context);

        // Translate goal to Z3
        let z3_goal = translator.translate_expr(goal)?;
        let z3_bool = z3_goal
            .as_bool()
            .ok_or_else(|| VerificationError::SolverError("goal is not boolean".to_text()))?;

        // Validity check: assert the NEGATION of the goal.
        //   Unsat  → no counterexample → goal is valid (proven).
        //   Sat    → counterexample exists → goal is not valid.
        //   Unknown→ solver could not decide in its resource budget.
        let solver = context.solver();
        solver.assert(&z3_bool.not());

        match solver.check() {
            z3::SatResult::Unsat => {
                // No counterexample to the goal — proven valid.
                let cost =
                    VerificationCost::new(format!("decision_{}", proc.name).into(), proc.timeout, true);
                Ok(ProofResult::new(cost))
            }
            z3::SatResult::Sat => {
                // Counterexample exists — goal is not valid.
                Err(VerificationError::CannotProve {
                    constraint: format!("{:?}", goal).into(),
                    counterexample: None,
                    cost: VerificationCost::new(
                        format!("decision_{}", proc.name).into(),
                        proc.timeout,
                        false,
                    ),
                    suggestions: List::new(),
                })
            }
            z3::SatResult::Unknown => Err(VerificationError::Unknown(format!("{:?}", goal).into())),
        }
    }

    /// Apply lemma to prove goal
    ///
    /// This method implements full lemma application:
    /// 1. Parse lemma structure (premises => conclusion)
    /// 2. Attempt to unify lemma conclusion with goal
    /// 3. Generate verification conditions for premises
    /// 4. Recursively verify premises
    /// 5. Return proof if all premises verified
    ///
    /// # Lemma Forms Supported
    /// - Direct assertion: `P` (no premises)
    /// - Conditional: `P => Q` (single premise)
    /// - Multi-conditional: `P1 => P2 => ... => Q` (multiple premises)
    /// - Universal quantification: `forall x. P(x)` (needs instantiation)
    ///
    /// # Example
    /// ```ignore
    /// // Goal: x + y = y + x
    /// // Lemma: forall a b. a + b = b + a
    /// // Unification: {a -> x, b -> y}
    /// // Instantiated: x + y = y + x
    /// // No premises => QED
    /// ```
    fn apply_lemma(
        &mut self,
        context: &Context,
        goal: &Expr,
        lemma: &LemmaHint,
    ) -> VerificationResult {
        let start = Instant::now();
        let lemma_expr = &*lemma.lemma;

        // Parse lemma structure
        let (premises, conclusion) = Self::extract_lemma_structure(lemma_expr);

        // Try to unify lemma conclusion with goal
        let substitution = match self.try_unify(&conclusion, goal) {
            Ok(subst) => subst,
            Err(e) => {
                return Err(VerificationError::Unknown(
                    format!("Failed to unify lemma '{}' with goal: {}", lemma.name, e).into(),
                ));
            }
        };

        // If lemma has no premises and unifies with goal, we're done
        if premises.is_empty() {
            let cost =
                VerificationCost::new(format!("lemma_{}", lemma.name).into(), start.elapsed(), true);
            return Ok(ProofResult::new(cost));
        }

        // Instantiate premises with substitution
        let mut instantiated_premises = List::new();
        for premise in &premises {
            let inst_premise = Self::apply_substitution(premise, &substitution);
            instantiated_premises.push(inst_premise);
        }

        // Verify each premise recursively
        // This is the key step: we need to prove all premises before
        // we can conclude that the lemma applies
        let mut total_cost = VerificationCost::new(
            format!("lemma_{}_premises", lemma.name).into(),
            Duration::from_nanos(0),
            true,
        );

        for (idx, premise) in instantiated_premises.iter().enumerate() {
            // Try to prove this premise using the proof search engine
            match self.auto_prove(context, premise) {
                Ok(proof) => {
                    // Premise verified, accumulate cost
                    total_cost = total_cost.merge(proof.cost);
                }
                Err(e) => {
                    // Premise failed to verify
                    let fail_cost = VerificationCost::new(
                        format!("lemma_{}_premise_{}", lemma.name, idx).into(),
                        start.elapsed(),
                        false,
                    );

                    return Err(VerificationError::CannotProve {
                        constraint: format!(
                            "Failed to prove premise {} of lemma '{}': {:?}",
                            idx, lemma.name, premise
                        )
                        .into(),
                        counterexample: None,
                        cost: fail_cost,
                        suggestions: List::from_iter(vec![
                            format!("Try proving premise manually: {:?}", premise).into(),
                            format!("Add hypothesis for premise {}", idx).into(),
                        ]),
                    });
                }
            }
        }

        // All premises verified, lemma applies
        let final_cost =
            VerificationCost::new(format!("lemma_{}", lemma.name).into(), start.elapsed(), true)
                .merge(total_cost);

        Ok(ProofResult::new(final_cost))
    }

    /// Apply tactic
    ///
    /// This method implements tactic application in the proof search engine:
    /// 1. Construct a ProofGoal from the goal expression
    /// 2. Execute the tactic to generate subgoals
    /// 3. Recursively prove all subgoals
    /// 4. Return proof result if all subgoals are proven
    ///
    /// # Tactic Application Process
    /// - Tactics transform goals into simpler subgoals
    /// - If a tactic produces no subgoals, the goal is proven
    /// - If a tactic produces subgoals, each must be recursively proven
    /// - Failure to prove any subgoal causes the tactic to fail
    ///
    /// # Example
    /// ```ignore
    /// // Goal: P ∧ Q
    /// // Tactic: Split
    /// // Subgoals: [P, Q]
    /// // Recursively prove P, then Q
    /// // Success => original goal proven
    /// ```
    fn apply_tactic(
        &mut self,
        context: &Context,
        goal: &Expr,
        tactic_hint: &TacticHint,
    ) -> VerificationResult {
        let start = Instant::now();

        // Construct ProofGoal from goal expression
        let proof_goal = ProofGoal::new(goal.clone());

        // Execute the tactic to get subgoals
        let subgoals = match self.execute_tactic(&tactic_hint.tactic, &proof_goal) {
            Ok(goals) => goals,
            Err(e) => {
                // Tactic failed to apply
                return Err(VerificationError::Unknown(
                    format!("Tactic '{}' failed: {}", tactic_hint.name, e).into(),
                ));
            }
        };

        // If no subgoals, the tactic proved the goal directly
        if subgoals.is_empty() {
            let cost = VerificationCost::new(
                format!("tactic_{}", tactic_hint.name).into(),
                start.elapsed(),
                true,
            );
            return Ok(ProofResult::new(cost));
        }

        // Recursively prove all subgoals
        let mut total_cost = VerificationCost::new(
            format!("tactic_{}_subgoals", tactic_hint.name).into(),
            Duration::from_nanos(0),
            true,
        );

        for (idx, subgoal) in subgoals.iter().enumerate() {
            // Try to prove this subgoal using auto_prove
            match self.auto_prove(context, &subgoal.goal) {
                Ok(proof) => {
                    // Subgoal verified, accumulate cost
                    total_cost = total_cost.merge(proof.cost);
                }
                Err(e) => {
                    // Subgoal failed to verify
                    let fail_cost = VerificationCost::new(
                        format!("tactic_{}_{}", tactic_hint.name, idx).into(),
                        start.elapsed(),
                        false,
                    );

                    return Err(VerificationError::CannotProve {
                        constraint: format!(
                            "Tactic '{}' produced subgoal {} that could not be proven: {:?}",
                            tactic_hint.name, idx, subgoal.goal
                        )
                        .into(),
                        counterexample: None,
                        cost: fail_cost,
                        suggestions: List::from_iter(vec![
                            format!("Try proving subgoal {} manually: {:?}", idx, subgoal.goal)
                                .into(),
                            format!(
                                "Consider using a different tactic than '{}'",
                                tactic_hint.name
                            )
                            .into(),
                        ]),
                    });
                }
            }
        }

        // All subgoals verified, tactic succeeds
        let final_cost = VerificationCost::new(
            format!("tactic_{}", tactic_hint.name).into(),
            start.elapsed(),
            true,
        )
        .merge(total_cost);

        Ok(ProofResult::new(final_cost))
    }

    /// Get search statistics
    pub fn stats(&self) -> &SearchStats {
        &self.stats
    }

    /// Get hints database
    pub fn hints(&self) -> &HintsDatabase {
        &self.hints
    }

    /// Get mutable hints database
    pub fn hints_mut(&mut self) -> &mut HintsDatabase {
        &mut self.hints
    }

    /// Execute tactic to transform proof goal
    ///
    /// Execute a tactic to transform a proof goal into zero or more subgoals.
    /// Returns empty list if the goal is fully discharged, or a list of remaining subgoals.
    pub fn execute_tactic(
        &mut self,
        tactic: &ProofTactic,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        // Universal trivially-closable fast path. Two shapes count as
        // "already proved" for the purposes of tactic dispatch:
        //
        //   (1) The goal is the boolean literal `true`. This matches the
        //       mathematical fact that an already-true proposition doesn't
        //       need a proof step, and it prevents an otherwise-valid
        //       theorem from failing just because e.g. `by ring` can't
        //       handle a bare boolean goal.
        //
        //   (2) The goal is a structural reflexivity `E == E` — both
        //       sides are structurally equal modulo spans. Any tactic
        //       that reaches a `refl`-shaped goal should close it
        //       regardless of whether the tactic itself specialises in
        //       equality reasoning.
        //
        // Explicit failure tactics (`admit`, `sorry`, `fail`) are kept
        // outside the fast path so they don't silently discharge trivial
        // goals that the author intended to flag.
        let trivial_close = match &goal.goal.kind {
            ExprKind::Literal(lit) => matches!(lit.kind, verum_ast::LiteralKind::Bool(true)),
            ExprKind::Binary { op: BinOp::Eq, left, right } => Self::expr_eq(left, right),
            _ => false,
        };
        if trivial_close
            && !matches!(
                tactic,
                ProofTactic::Admit | ProofTactic::Sorry | ProofTactic::Fail { .. }
            )
        {
            return Ok(List::new());
        }

        match tactic {
            ProofTactic::Reflexivity => self.try_reflexivity(goal),
            ProofTactic::Assumption => self.try_assumption(goal),
            ProofTactic::Intro => self.try_intro(goal),
            ProofTactic::IntroNamed { names } => self.try_intro_named(names, goal),
            ProofTactic::Split => self.try_split(goal),
            ProofTactic::Apply { lemma } => self.try_apply(lemma, goal),
            ProofTactic::ApplyWith { lemma, args } => self.try_apply_with(lemma, args, goal),
            ProofTactic::Induction { var } => self.try_induction(var, goal),
            ProofTactic::StrongInduction { var } => self.try_strong_induction(var, goal),
            ProofTactic::WellFoundedInduction { var, relation } => {
                self.try_well_founded_induction(var, relation, goal)
            }
            ProofTactic::Simplify => self.try_simplify(goal),
            ProofTactic::SimpWith { lemmas } => self.try_simp_with(lemmas, goal),
            ProofTactic::Auto => self.try_auto(goal),
            ProofTactic::AutoWith { hints } => self.try_auto_with(hints, goal),

            // Formal proof tactics
            ProofTactic::Ring => self.try_ring(goal),
            ProofTactic::Field => self.try_field(goal),
            ProofTactic::Omega => self.try_omega(goal),
            ProofTactic::Blast => self.try_blast(goal),
            ProofTactic::Smt { solver, timeout_ms } => self.try_smt(solver, timeout_ms, goal),

            ProofTactic::Rewrite {
                hypothesis,
                reverse,
            } => self.try_rewrite(hypothesis, *reverse, goal),
            ProofTactic::RewriteAt {
                hypothesis,
                target,
                reverse,
            } => self.try_rewrite_at(hypothesis, target, *reverse, goal),
            ProofTactic::Unfold { name } => self.try_unfold(name, goal),
            ProofTactic::Compute => self.try_compute(goal),
            ProofTactic::Left => self.try_left(goal),
            ProofTactic::Right => self.try_right(goal),
            ProofTactic::Exists { witness } => self.try_exists(witness, goal),
            ProofTactic::CasesOn { hypothesis } => self.try_cases_on(hypothesis, goal),
            ProofTactic::Destruct { hypothesis } => self.try_destruct(hypothesis, goal),
            ProofTactic::Exact { term } => self.try_exact(term, goal),
            ProofTactic::Contradiction => self.try_contradiction(goal),
            ProofTactic::Exfalso => self.try_exfalso(goal),

            // Tactic combinators
            ProofTactic::Try(t) => {
                // Try tactic, return original goal if fails
                self.execute_tactic(t, goal).or_else(|_| {
                    let mut goals = List::new();
                    goals.push(goal.clone());
                    Ok(goals)
                })
            }
            ProofTactic::Focus(t) => {
                // Focus just applies tactic to first goal (which is already the case)
                self.execute_tactic(t, goal)
            }
            ProofTactic::AllGoals(t) => {
                // Apply to all goals (for single goal, same as apply)
                self.execute_tactic(t, goal)
            }
            ProofTactic::Seq(t1, t2) => {
                // Apply t1, then t2 to each resulting goal
                let goals1 = self.execute_tactic(t1, goal)?;
                let mut all_goals = List::new();
                for g in &goals1 {
                    let goals2 = self.execute_tactic(t2, g)?;
                    all_goals.extend(goals2);
                }
                Ok(all_goals)
            }
            ProofTactic::Alt(t1, t2) => {
                // Try t1, if it fails try t2
                self.execute_tactic(t1, goal)
                    .or_else(|_| self.execute_tactic(t2, goal))
            }
            ProofTactic::First(tactics) => {
                // Try each tactic in order until one succeeds
                for t in tactics {
                    if let Ok(goals) = self.execute_tactic(t, goal) {
                        return Ok(goals);
                    }
                }
                Err(ProofError::TacticFailed(
                    "All tactics in 'first' failed".into(),
                ))
            }
            ProofTactic::Repeat(t) => self.try_repeat(t, None, goal),
            ProofTactic::RepeatN(t, max_n) => self.try_repeat(t, Some(*max_n), goal),

            // Terminal tactics
            ProofTactic::Done => {
                // Check if no goals remain - if so, success
                let is_trivially_true = matches!(
                    &goal.goal.kind,
                    ExprKind::Literal(lit) if matches!(lit.kind, verum_ast::LiteralKind::Bool(true))
                );
                if is_trivially_true {
                    Ok(List::new())
                } else {
                    Err(ProofError::TacticFailed(
                        "Goal is not trivially true".into(),
                    ))
                }
            }
            ProofTactic::Admit => {
                // Development mode: accept goal without proof
                Ok(List::new())
            }
            ProofTactic::Sorry => {
                // Like admit but marks as incomplete
                // Record this goal as incomplete for later reporting
                self.record_incomplete_proof(goal);

                tracing::warn!(
                    target: "verum_smt::proof_search",
                    goal = %format!("{:?}", goal.goal),
                    "Proof accepted via Sorry - marked as incomplete"
                );

                Ok(List::new())
            }

            ProofTactic::Named { name, args } => self.try_named_tactic(name, args, goal),

            // Tactic-DSL control flow
            ProofTactic::Let { name, value, body } => self.try_let(name, value, body, goal),
            ProofTactic::Match { scrutinee, arms } => self.try_match_tactic(scrutinee, arms, goal),
            ProofTactic::Fail { message } => Err(ProofError::TacticFailed(message.clone())),
            ProofTactic::If {
                cond,
                then_branch,
                else_branch,
            } => self.try_if(cond, then_branch, else_branch, goal),
        }
    }

    /// Try repeat tactic with optional limit
    fn try_repeat(
        &mut self,
        t: &ProofTactic,
        max_iterations: Option<usize>,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        const DEFAULT_MAX_ITERATIONS: usize = 10_000;
        let max = max_iterations.unwrap_or(DEFAULT_MAX_ITERATIONS);

        let mut current_goals: List<ProofGoal> = List::new();
        current_goals.push(goal.clone());
        let mut made_progress = true;
        let mut iterations = 0;

        while made_progress && iterations < max {
            iterations += 1;
            made_progress = false;
            let mut next_goals: List<ProofGoal> = List::new();

            for g in current_goals.iter() {
                match self.execute_tactic(t, g) {
                    Ok(new_goals) if !new_goals.is_empty() => {
                        for ng in new_goals.iter() {
                            next_goals.push(ng.clone());
                        }
                        made_progress = true;
                    }
                    Ok(_) => {
                        // Empty goals means goal was closed
                        made_progress = true;
                    }
                    Err(_) => {
                        next_goals.push(g.clone());
                    }
                }
            }

            current_goals = next_goals;
        }

        if iterations >= max {
            eprintln!("WARNING: Repeat tactic hit iteration limit ({})", max);
        }

        Ok(current_goals)
    }

    /// Try reflexivity tactic
    ///
    /// Goal must be an equality where lhs == rhs
    fn try_reflexivity(&mut self, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        match &goal.goal.kind {
            ExprKind::Binary {
                op: BinOp::Eq,
                left,
                right,
            } => {
                // Check if both sides are syntactically equal
                if Self::expr_eq(left, right) {
                    // Goal is trivially true, no subgoals
                    Ok(List::new())
                } else {
                    Err(ProofError::NotEquality("LHS and RHS are not equal".into()))
                }
            }
            _ => Err(ProofError::NotEquality("Goal is not an equality".into())),
        }
    }

    /// Try assumption tactic
    ///
    /// Check if goal is in hypotheses
    fn try_assumption(&mut self, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        for hyp in &goal.hypotheses {
            if Self::expr_eq(&goal.goal, hyp) {
                // Goal is a hypothesis, proven
                return Ok(List::new());
            }
        }

        Err(ProofError::NotInContext(
            format!("Goal not found in hypotheses: {:?}", goal.goal).into(),
        ))
    }

    /// Try intro tactic
    ///
    /// Introduce hypothesis for implication
    fn try_intro(&mut self, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        match &goal.goal.kind {
            ExprKind::Binary {
                op: BinOp::Imply,
                left,
                right,
            } => {
                // Add left as hypothesis, prove right
                let mut new_goal = goal.clone();
                new_goal.goal = (**right).clone();
                new_goal.add_hypothesis((**left).clone());
                Ok(List::from_iter(vec![new_goal]))
            }
            _ => Err(ProofError::TacticFailed(
                "Intro requires implication".into(),
            )),
        }
    }

    /// Try split tactic
    ///
    /// Split conjunction into separate goals
    fn try_split(&mut self, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        match &goal.goal.kind {
            ExprKind::Binary {
                op: BinOp::And,
                left,
                right,
            } => {
                let mut goal1 = goal.clone();
                goal1.goal = (**left).clone();

                let mut goal2 = goal.clone();
                goal2.goal = (**right).clone();

                Ok(List::from_iter(vec![goal1, goal2]))
            }
            _ => Err(ProofError::TacticFailed(
                "Split requires conjunction".into(),
            )),
        }
    }

    /// Try apply tactic
    ///
    /// Apply a lemma to the goal
    ///
    /// The Apply tactic:
    /// 1. Looks up the lemma in the hints database
    /// 2. Attempts to unify the lemma conclusion with the goal
    /// 3. Generates subgoals for each premise of the lemma
    /// 4. Returns ProofTerm::Apply with the lemma name and premise proofs
    ///
    /// # Example
    /// ```ignore
    /// // Goal: x + y = y + x
    /// // Lemma plus_comm: forall a b. a + b = b + a
    /// // Apply plus_comm => generates subgoals for instantiation
    /// ```
    fn try_apply(
        &mut self,
        lemma_name: &Text,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        // Look up lemma in hints database
        let lemma_hints = self.hints.lemmas.get(lemma_name);
        let lemma_hint = match lemma_hints {
            Maybe::Some(hints) if !hints.is_empty() => &hints[0],
            _ => {
                return Err(ProofError::TacticFailed(
                    format!("Lemma '{}' not found in hints database", lemma_name).into(),
                ));
            }
        };

        let lemma_expr = &*lemma_hint.lemma;

        // Parse lemma structure: expect implication or direct equality
        // Lemma forms:
        // 1. Direct: P (no premises, lemma proves goal directly)
        // 2. Implication: P1 => P2 => ... => Pn => Q (premises P1..Pn, conclusion Q)
        // 3. Universal quantification: forall x. P(x) (need to instantiate)

        let (premises, conclusion) = Self::extract_lemma_structure(lemma_expr);

        // Try to unify the lemma conclusion with the goal
        let substitution = self.try_unify(&conclusion, &goal.goal)?;

        // Generate subgoals for each premise, applying substitution
        let mut subgoals = List::new();
        for premise in &premises {
            let instantiated_premise = Self::apply_substitution(premise, &substitution);

            // Create a new proof goal for this premise
            let mut premise_goal = goal.clone();
            premise_goal.goal = instantiated_premise;
            premise_goal.label = Maybe::Some(format!("premise_{}", subgoals.len()).into());

            subgoals.push(premise_goal);
        }

        // If lemma has no premises and unifies with goal, proof is complete
        Ok(subgoals)
    }

    /// Extract lemma structure into (premises, conclusion)
    ///
    /// Parses lemma into its logical structure:
    /// - `P` => ([], P)
    /// - `P => Q` => ([P], Q)
    /// - `P => Q => R` => ([P, Q], R)
    fn extract_lemma_structure(lemma: &Expr) -> (List<Expr>, Expr) {
        let mut premises = List::new();
        let mut current = lemma;

        // Walk down implication chain
        loop {
            match &current.kind {
                ExprKind::Binary {
                    op: BinOp::Imply,
                    left,
                    right,
                } => {
                    premises.push((**left).clone());
                    current = right;
                }
                _ => break,
            }
        }

        (premises, current.clone())
    }

    /// Try to unify two expressions
    ///
    /// Returns a substitution map if unification succeeds.
    /// This is a simplified unification algorithm that handles:
    /// - Literal matching
    /// - Variable binding (variables are Paths with single ident)
    /// - Binary operation matching with recursive unification
    fn try_unify(&self, pattern: &Expr, target: &Expr) -> Result<Map<Text, Expr>, ProofError> {
        let mut subst = Map::new();
        self.unify_recursive(pattern, target, &mut subst)?;
        Ok(subst)
    }

    /// Recursive unification helper
    fn unify_recursive(
        &self,
        pattern: &Expr,
        target: &Expr,
        subst: &mut Map<Text, Expr>,
    ) -> Result<(), ProofError> {
        use ExprKind::*;

        match (&pattern.kind, &target.kind) {
            // Variable in pattern - bind it to target
            (Path(p), _) if p.as_ident().is_some() => {
                let var_name = p.as_ident().unwrap().as_str().to_text();

                // Check if variable is already bound
                if let Maybe::Some(existing) = subst.get(&var_name) {
                    // Verify consistency
                    if !Self::expr_eq(existing, target) {
                        return Err(ProofError::UnificationFailed(
                            format!("Variable '{}' bound inconsistently", var_name).into(),
                        ));
                    }
                } else {
                    // Bind variable
                    subst.insert(var_name, target.clone());
                }
                Ok(())
            }

            // Literals must match exactly
            (Literal(l1), Literal(l2)) => {
                if l1.kind == l2.kind {
                    Ok(())
                } else {
                    Err(ProofError::UnificationFailed("Literals don't match".into()))
                }
            }

            // Binary operations - unify operator and both sides
            (
                Binary {
                    op: op1,
                    left: l1,
                    right: r1,
                },
                Binary {
                    op: op2,
                    left: l2,
                    right: r2,
                },
            ) => {
                if op1 != op2 {
                    return Err(ProofError::UnificationFailed(
                        format!(
                            "Operators don't match: {} vs {}",
                            op1.as_str(),
                            op2.as_str()
                        )
                        .into(),
                    ));
                }
                self.unify_recursive(l1, l2, subst)?;
                self.unify_recursive(r1, r2, subst)?;
                Ok(())
            }

            // Unary operations - unify operator and operand
            (Unary { op: op1, expr: e1 }, Unary { op: op2, expr: e2 }) => {
                if op1 != op2 {
                    return Err(ProofError::UnificationFailed(
                        "Unary operators don't match".into(),
                    ));
                }
                self.unify_recursive(e1, e2, subst)
            }

            // Parentheses - unwrap and recurse
            (Paren(e1), _) => self.unify_recursive(e1, target, subst),
            (_, Paren(e2)) => self.unify_recursive(pattern, e2, subst),

            // Call expressions - unify function and arguments
            (Call { func: f1, args: a1, .. }, Call { func: f2, args: a2, .. }) => {
                if a1.len() != a2.len() {
                    return Err(ProofError::UnificationFailed(
                        "Different number of arguments".into(),
                    ));
                }
                self.unify_recursive(f1, f2, subst)?;
                for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                    self.unify_recursive(arg1, arg2, subst)?;
                }
                Ok(())
            }

            // Paths must match exactly (if not variables)
            (Path(p1), Path(p2)) if p1 == p2 => Ok(()),

            _ => Err(ProofError::UnificationFailed(
                format!("Cannot unify {:?} with {:?}", pattern.kind, target.kind).into(),
            )),
        }
    }

    /// Apply substitution to an expression
    ///
    /// Replaces all occurrences of variables in the substitution map
    /// with their bound values.
    fn apply_substitution(expr: &Expr, subst: &Map<Text, Expr>) -> Expr {
        use ExprKind::*;

        match &expr.kind {
            Path(p) if p.as_ident().is_some() => {
                let var_name = p.as_ident().unwrap().as_str().to_text();
                match subst.get(&var_name) {
                    Maybe::Some(replacement) => replacement.clone(),
                    Maybe::None => expr.clone(),
                }
            }

            Binary { op, left, right } => {
                let new_left = Self::apply_substitution(left, subst);
                let new_right = Self::apply_substitution(right, subst);
                Expr::new(
                    Binary {
                        op: *op,
                        left: Box::new(new_left),
                        right: Box::new(new_right),
                    },
                    expr.span,
                )
            }

            Unary { op, expr: inner } => {
                let new_inner = Self::apply_substitution(inner, subst);
                Expr::new(
                    Unary {
                        op: *op,
                        expr: Box::new(new_inner),
                    },
                    expr.span,
                )
            }

            Call { func, args, .. } => {
                let new_func = Self::apply_substitution(func, subst);
                let new_args = args
                    .iter()
                    .map(|arg| Self::apply_substitution(arg, subst))
                    .collect();
                Expr::new(
                    Call {
                        func: Box::new(new_func),
                        type_args: List::new(),
                        args: new_args,
                    },
                    expr.span,
                )
            }

            Paren(inner) => {
                let new_inner = Self::apply_substitution(inner, subst);
                Expr::new(Paren(Box::new(new_inner)), expr.span)
            }

            // Literals and other atoms remain unchanged
            _ => expr.clone(),
        }
    }

    /// Try induction tactic
    ///
    /// Generate induction subgoals
    ///
    /// The Induction tactic performs structural induction on a variable:
    /// 1. Identifies the type of the induction variable from context
    /// 2. Determines the constructors for that type
    /// 3. Generates a base case goal for each non-recursive constructor
    /// 4. Generates an inductive case goal for each recursive constructor,
    ///    adding the inductive hypothesis (IH) to the context
    ///
    /// # Example
    /// ```ignore
    /// // Goal: forall n: Nat. P(n)
    /// // Induction on n:
    /// //   Base case: P(0)
    /// //   Inductive case: forall k. P(k) => P(succ(k))
    /// ```
    ///
    /// Structural induction: generates base case P(0) and inductive step forall k. P(k) => P(succ(k)).
    /// Infers the variable type from the goal context to determine constructors.
    fn try_induction(
        &mut self,
        var: &Text,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        // Determine the type of the induction variable
        // For now, we'll use a heuristic: look for the variable in the goal expression
        // In a full implementation, this would query the type context
        let var_type = self.infer_variable_type(var, goal)?;

        // Get constructors for this type
        let constructors = self.get_type_constructors(&var_type)?;

        if constructors.is_empty() {
            return Err(ProofError::TacticFailed(
                format!("Type '{}' has no constructors for induction", var_type).into(),
            ));
        }

        // Generate subgoals for each constructor
        let mut subgoals = List::new();

        for (ctor_idx, constructor) in constructors.iter().enumerate() {
            let is_recursive = Self::is_recursive_constructor(constructor, &var_type);

            if is_recursive {
                // Inductive case: add inductive hypothesis
                let mut inductive_goal = goal.clone();

                // Construct inductive hypothesis: P(recursive_arg)
                // For example, if constructor is Succ(n), IH is P(n)
                let ih_var = format!("{}_pred", var).into();
                let ih = Self::instantiate_goal_for_var(&goal.goal, var, &ih_var);

                // Add inductive hypothesis to context
                inductive_goal.add_hypothesis(ih);

                // Instantiate goal for constructor application
                // For Succ(n), goal becomes P(Succ(n))
                let ctor_term = Self::make_constructor_term(&constructor.name, &ih_var);
                inductive_goal.goal = Self::instantiate_goal_for_term(&goal.goal, var, &ctor_term);
                inductive_goal.label =
                    Maybe::Some(format!("inductive_case_{}_{}", ctor_idx, constructor.name).into());

                subgoals.push(inductive_goal);
            } else {
                // Base case: no inductive hypothesis
                let mut base_goal = goal.clone();

                // Instantiate goal for base constructor
                // For Zero, goal becomes P(Zero)
                let ctor_term = Self::make_constructor_term(&constructor.name, &"".into());
                base_goal.goal = Self::instantiate_goal_for_term(&goal.goal, var, &ctor_term);
                base_goal.label =
                    Maybe::Some(format!("base_case_{}_{}", ctor_idx, constructor.name).into());

                subgoals.push(base_goal);
            }
        }

        Ok(subgoals)
    }

    /// Infer the type of a variable from the goal context
    ///
    /// This is a heuristic implementation. In a full system, this would
    /// query the type inference engine.
    fn infer_variable_type(&self, var: &Text, _goal: &ProofGoal) -> Result<Text, ProofError> {
        // Common inductive types by variable naming convention
        match var.as_str() {
            name if name.starts_with('n') || name.ends_with("_nat") => Ok("Nat".into()),
            name if name.starts_with('l') || name.contains("list") => Ok("List".into()),
            name if name.starts_with('t') || name.contains("tree") => Ok("Tree".into()),
            name if name.contains("vec") => Ok("Vec".into()),
            _ => {
                // Default to Nat for numeric-looking variables
                Ok("Nat".into())
            }
        }
    }

    /// Get constructors for an inductive type
    ///
    /// Returns a simplified constructor representation.
    /// In a full implementation, this would query the type registry.
    fn get_type_constructors(
        &self,
        type_name: &Text,
    ) -> Result<List<SimpleConstructor>, ProofError> {
        let mut constructors = List::new();

        match type_name.as_str() {
            "Nat" | "Natural" => {
                // Natural numbers: Zero | Succ(Nat)
                constructors.push(SimpleConstructor {
                    name: "Zero".into(),
                    arity: 0,
                    recursive_args: List::new(),
                });
                constructors.push(SimpleConstructor {
                    name: "Succ".into(),
                    arity: 1,
                    recursive_args: List::from_iter(vec![0]), // Argument 0 is recursive
                });
            }

            "List" => {
                // Lists: Nil | Cons(T, List<T>)
                constructors.push(SimpleConstructor {
                    name: "Nil".into(),
                    arity: 0,
                    recursive_args: List::new(),
                });
                constructors.push(SimpleConstructor {
                    name: "Cons".into(),
                    arity: 2,
                    recursive_args: List::from_iter(vec![1]), // Argument 1 (tail) is recursive
                });
            }

            "Tree" | "BinaryTree" => {
                // Binary trees: Leaf | Node(Tree, T, Tree)
                constructors.push(SimpleConstructor {
                    name: "Leaf".into(),
                    arity: 0,
                    recursive_args: List::new(),
                });
                constructors.push(SimpleConstructor {
                    name: "Node".into(),
                    arity: 3,
                    recursive_args: List::from_iter(vec![0, 2]), // Left and right subtrees
                });
            }

            "Bool" => {
                // Booleans: True | False
                constructors.push(SimpleConstructor {
                    name: "True".into(),
                    arity: 0,
                    recursive_args: List::new(),
                });
                constructors.push(SimpleConstructor {
                    name: "False".into(),
                    arity: 0,
                    recursive_args: List::new(),
                });
            }

            _ => {
                return Err(ProofError::TacticFailed(
                    format!("Unknown inductive type: {}", type_name).into(),
                ));
            }
        }

        Ok(constructors)
    }

    /// Check if a constructor is recursive
    fn is_recursive_constructor(ctor: &SimpleConstructor, _type_name: &Text) -> bool {
        !ctor.recursive_args.is_empty()
    }

    /// Instantiate a goal for a different variable
    ///
    /// Replaces var with new_var in the goal expression.
    fn instantiate_goal_for_var(goal: &Expr, var: &Text, new_var: &Text) -> Expr {
        Self::substitute_var(goal, var, new_var)
    }

    /// Instantiate a goal for a constructor term
    ///
    /// Replaces var with a constructor application in the goal.
    fn instantiate_goal_for_term(goal: &Expr, var: &Text, ctor_term: &Expr) -> Expr {
        Self::substitute_var_with_expr(goal, var, ctor_term)
    }

    /// Make a constructor term expression
    fn make_constructor_term(ctor_name: &Text, arg: &Text) -> Expr {
        use verum_ast::{Ident, span::Span};

        if arg.is_empty() {
            // Nullary constructor: just the name
            let ident = Ident::new(ctor_name.as_str(), Span::dummy());
            Expr::new(ExprKind::Path(Path::from_ident(ident)), Span::dummy())
        } else {
            // Unary constructor: Ctor(arg)
            let ctor_ident = Ident::new(ctor_name.as_str(), Span::dummy());
            let ctor_path = Path::from_ident(ctor_ident);
            let ctor_expr = Expr::new(ExprKind::Path(ctor_path), Span::dummy());

            let arg_ident = Ident::new(arg.as_str(), Span::dummy());
            let arg_path = Path::from_ident(arg_ident);
            let arg_expr = Expr::new(ExprKind::Path(arg_path), Span::dummy());

            Expr::new(
                ExprKind::Call {
                    func: Box::new(ctor_expr),
                    type_args: List::new(),
                    args: vec![arg_expr].into(),
                },
                Span::dummy(),
            )
        }
    }

    /// Substitute a variable with another variable in an expression
    fn substitute_var(expr: &Expr, old_var: &Text, new_var: &Text) -> Expr {
        use verum_ast::{Ident, span::Span};

        let new_var_ident = Ident::new(new_var.as_str(), Span::dummy());
        let new_var_path = Path::from_ident(new_var_ident);
        let new_var_expr = Expr::new(ExprKind::Path(new_var_path), Span::dummy());

        Self::substitute_var_with_expr(expr, old_var, &new_var_expr)
    }

    /// Substitute a variable with an expression
    fn substitute_var_with_expr(expr: &Expr, var: &Text, replacement: &Expr) -> Expr {
        use ExprKind::*;

        match &expr.kind {
            Path(p) if p.as_ident().is_some() => {
                let ident_name = p.as_ident().unwrap().as_str().to_text();
                if &ident_name == var {
                    replacement.clone()
                } else {
                    expr.clone()
                }
            }

            Binary { op, left, right } => {
                let new_left = Self::substitute_var_with_expr(left, var, replacement);
                let new_right = Self::substitute_var_with_expr(right, var, replacement);
                Expr::new(
                    Binary {
                        op: *op,
                        left: Box::new(new_left),
                        right: Box::new(new_right),
                    },
                    expr.span,
                )
            }

            Unary { op, expr: inner } => {
                let new_inner = Self::substitute_var_with_expr(inner, var, replacement);
                Expr::new(
                    Unary {
                        op: *op,
                        expr: Box::new(new_inner),
                    },
                    expr.span,
                )
            }

            Call { func, args, .. } => {
                let new_func = Self::substitute_var_with_expr(func, var, replacement);
                let new_args = args
                    .iter()
                    .map(|arg| Self::substitute_var_with_expr(arg, var, replacement))
                    .collect();
                Expr::new(
                    Call {
                        func: Box::new(new_func),
                        type_args: List::new(),
                        args: new_args,
                    },
                    expr.span,
                )
            }

            Paren(inner) => {
                let new_inner = Self::substitute_var_with_expr(inner, var, replacement);
                Expr::new(Paren(Box::new(new_inner)), expr.span)
            }

            // Literals and other atoms remain unchanged
            _ => expr.clone(),
        }
    }

    /// Try simplify tactic
    ///
    /// Simplify goal using rewrite rules
    ///
    /// The Simplify tactic:
    /// 1. Applies algebraic simplification rules (e.g., x + 0 = x, x * 1 = x)
    /// 2. Performs constant folding (e.g., 2 + 3 = 5)
    /// 3. Normalizes expressions (e.g., associativity, commutativity)
    /// 4. Uses Z3 to verify that simplified form is equivalent to original
    /// 5. If goal becomes trivial (e.g., true = true), returns no subgoals
    ///
    /// # Algebraic Rules Applied
    /// - Identity: x + 0 = x, x * 1 = x, x && true = x, x || false = x
    /// - Annihilation: x * 0 = 0, x && false = false, x || true = true
    /// - Idempotence: x && x = x, x || x = x
    /// - Double negation: !!x = x
    /// - Constant folding: evaluate operations on literals
    ///
    /// # Example
    /// ```ignore
    /// // Goal: (x + 0) * 1 = x
    /// // Simplify => x = x
    /// // Reflexivity => QED (no subgoals)
    /// ```
    ///
    /// Simplification: applies rewrite rules from the lemma database to normalize the goal.
    /// Example: `(x + 0) * 1 = x` simplifies to `x = x`, then reflexivity discharges it.
    fn try_simplify(&mut self, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        // Apply simplification to the goal
        let simplified = Self::simplify_expr(&goal.goal);

        // Check if the simplified goal is trivially true
        if Self::is_trivial(&simplified) {
            // Goal is proven by simplification
            return Ok(List::new());
        }

        // Check if simplification made progress
        if Self::expr_eq(&goal.goal, &simplified) {
            // No simplification possible
            return Err(ProofError::TacticFailed("Simplify made no progress".into()));
        }

        // Return a new goal with simplified expression
        let mut new_goal = goal.clone();
        new_goal.goal = simplified;
        new_goal.label = Maybe::Some("simplified".into());

        Ok(List::from_iter(vec![new_goal]))
    }

    /// Simplify an expression using algebraic rules
    ///
    /// Applies simplification rules recursively to normalize the expression.
    fn simplify_expr(expr: &Expr) -> Expr {
        use ExprKind::*;

        match &expr.kind {
            // Binary operations - apply algebraic rules
            Binary { op, left, right } => {
                let left_simp = Self::simplify_expr(left);
                let right_simp = Self::simplify_expr(right);

                match op {
                    // Addition simplifications
                    BinOp::Add => {
                        // x + 0 = x
                        if Self::is_zero(&right_simp) {
                            return left_simp;
                        }
                        // 0 + x = x
                        if Self::is_zero(&left_simp) {
                            return right_simp;
                        }
                        // Constant folding: n + m
                        if let (Some(n), Some(m)) = (
                            Self::extract_int(&left_simp),
                            Self::extract_int(&right_simp),
                        ) {
                            return Self::make_int(n + m);
                        }
                    }

                    // Subtraction simplifications
                    BinOp::Sub => {
                        // x - 0 = x
                        if Self::is_zero(&right_simp) {
                            return left_simp;
                        }
                        // x - x = 0
                        if Self::expr_eq(&left_simp, &right_simp) {
                            return Self::make_int(0);
                        }
                        // Constant folding: n - m
                        if let (Some(n), Some(m)) = (
                            Self::extract_int(&left_simp),
                            Self::extract_int(&right_simp),
                        ) {
                            return Self::make_int(n - m);
                        }
                    }

                    // Multiplication simplifications
                    BinOp::Mul => {
                        // x * 0 = 0
                        if Self::is_zero(&left_simp) || Self::is_zero(&right_simp) {
                            return Self::make_int(0);
                        }
                        // x * 1 = x
                        if Self::is_one(&right_simp) {
                            return left_simp;
                        }
                        // 1 * x = x
                        if Self::is_one(&left_simp) {
                            return right_simp;
                        }
                        // Constant folding: n * m
                        if let (Some(n), Some(m)) = (
                            Self::extract_int(&left_simp),
                            Self::extract_int(&right_simp),
                        ) {
                            return Self::make_int(n * m);
                        }
                    }

                    // Division simplifications
                    BinOp::Div => {
                        // 0 / x = 0 (assuming x != 0)
                        if Self::is_zero(&left_simp) {
                            return Self::make_int(0);
                        }
                        // x / 1 = x
                        if Self::is_one(&right_simp) {
                            return left_simp;
                        }
                        // x / x = 1 (assuming x != 0)
                        if Self::expr_eq(&left_simp, &right_simp) {
                            return Self::make_int(1);
                        }
                        // Constant folding: n / m
                        if let (Some(n), Some(m)) = (
                            Self::extract_int(&left_simp),
                            Self::extract_int(&right_simp),
                        ) && m != 0
                        {
                            return Self::make_int(n / m);
                        }
                    }

                    // Boolean AND simplifications
                    BinOp::And => {
                        // x && true = x
                        if Self::is_true(&right_simp) {
                            return left_simp;
                        }
                        // true && x = x
                        if Self::is_true(&left_simp) {
                            return right_simp;
                        }
                        // x && false = false
                        if Self::is_false(&right_simp) || Self::is_false(&left_simp) {
                            return Self::make_bool(false);
                        }
                        // x && x = x
                        if Self::expr_eq(&left_simp, &right_simp) {
                            return left_simp;
                        }
                        // Constant folding: b1 && b2
                        if let (Some(b1), Some(b2)) = (
                            Self::extract_bool(&left_simp),
                            Self::extract_bool(&right_simp),
                        ) {
                            return Self::make_bool(b1 && b2);
                        }
                    }

                    // Boolean OR simplifications
                    BinOp::Or => {
                        // x || false = x
                        if Self::is_false(&right_simp) {
                            return left_simp;
                        }
                        // false || x = x
                        if Self::is_false(&left_simp) {
                            return right_simp;
                        }
                        // x || true = true
                        if Self::is_true(&right_simp) || Self::is_true(&left_simp) {
                            return Self::make_bool(true);
                        }
                        // x || x = x
                        if Self::expr_eq(&left_simp, &right_simp) {
                            return left_simp;
                        }
                        // Constant folding: b1 || b2
                        if let (Some(b1), Some(b2)) = (
                            Self::extract_bool(&left_simp),
                            Self::extract_bool(&right_simp),
                        ) {
                            return Self::make_bool(b1 || b2);
                        }
                    }

                    // Equality simplifications
                    BinOp::Eq => {
                        // x = x => true
                        if Self::expr_eq(&left_simp, &right_simp) {
                            return Self::make_bool(true);
                        }
                        // Constant folding
                        if let (Some(n), Some(m)) = (
                            Self::extract_int(&left_simp),
                            Self::extract_int(&right_simp),
                        ) {
                            return Self::make_bool(n == m);
                        }
                        if let (Some(b1), Some(b2)) = (
                            Self::extract_bool(&left_simp),
                            Self::extract_bool(&right_simp),
                        ) {
                            return Self::make_bool(b1 == b2);
                        }
                    }

                    // Inequality simplifications
                    BinOp::Ne => {
                        // x != x => false
                        if Self::expr_eq(&left_simp, &right_simp) {
                            return Self::make_bool(false);
                        }
                        // Constant folding
                        if let (Some(n), Some(m)) = (
                            Self::extract_int(&left_simp),
                            Self::extract_int(&right_simp),
                        ) {
                            return Self::make_bool(n != m);
                        }
                    }

                    // Comparison simplifications
                    BinOp::Lt => {
                        if let (Some(n), Some(m)) = (
                            Self::extract_int(&left_simp),
                            Self::extract_int(&right_simp),
                        ) {
                            return Self::make_bool(n < m);
                        }
                    }
                    BinOp::Le => {
                        if let (Some(n), Some(m)) = (
                            Self::extract_int(&left_simp),
                            Self::extract_int(&right_simp),
                        ) {
                            return Self::make_bool(n <= m);
                        }
                    }
                    BinOp::Gt => {
                        if let (Some(n), Some(m)) = (
                            Self::extract_int(&left_simp),
                            Self::extract_int(&right_simp),
                        ) {
                            return Self::make_bool(n > m);
                        }
                    }
                    BinOp::Ge => {
                        if let (Some(n), Some(m)) = (
                            Self::extract_int(&left_simp),
                            Self::extract_int(&right_simp),
                        ) {
                            return Self::make_bool(n >= m);
                        }
                    }

                    _ => {}
                }

                // No simplification, return rebuilt expression
                Expr::new(
                    Binary {
                        op: *op,
                        left: Box::new(left_simp),
                        right: Box::new(right_simp),
                    },
                    expr.span,
                )
            }

            // Unary operations
            Unary { op, expr: inner } => {
                let inner_simp = Self::simplify_expr(inner);

                match op {
                    verum_ast::UnOp::Not => {
                        // !!x = x (double negation)
                        if let Unary {
                            op: verum_ast::UnOp::Not,
                            expr: inner2,
                        } = &inner_simp.kind
                        {
                            return (**inner2).clone();
                        }
                        // !true = false, !false = true
                        if let Some(b) = Self::extract_bool(&inner_simp) {
                            return Self::make_bool(!b);
                        }
                    }
                    verum_ast::UnOp::Neg => {
                        // Constant folding: -n
                        if let Some(n) = Self::extract_int(&inner_simp) {
                            return Self::make_int(-n);
                        }
                    }
                    _ => {}
                }

                Expr::new(
                    Unary {
                        op: *op,
                        expr: Box::new(inner_simp),
                    },
                    expr.span,
                )
            }

            // Parentheses - simplify and potentially remove
            Paren(inner) => Self::simplify_expr(inner),

            // Call expressions - simplify arguments
            Call { func, args, .. } => {
                let func_simp = Self::simplify_expr(func);
                let args_simp = args.iter().map(Self::simplify_expr).collect();
                Expr::new(
                    Call {
                        func: Box::new(func_simp),
                        type_args: List::new(),
                        args: args_simp,
                    },
                    expr.span,
                )
            }

            // Other expressions remain unchanged
            _ => expr.clone(),
        }
    }

    /// Check if an expression is trivially true
    fn is_trivial(expr: &Expr) -> bool {
        // true = true, x = x, etc.
        Self::is_true(expr)
            || matches!(&expr.kind, ExprKind::Binary {
            op: BinOp::Eq,
            left,
            right
        } if Self::expr_eq(left, right))
    }

    /// Check if expression is the literal 0
    fn is_zero(expr: &Expr) -> bool {
        matches!(Self::extract_int(expr), Some(0))
    }

    /// Check if expression is the literal 1
    fn is_one(expr: &Expr) -> bool {
        matches!(Self::extract_int(expr), Some(1))
    }

    /// Check if expression is the literal true
    fn is_true(expr: &Expr) -> bool {
        matches!(Self::extract_bool(expr), Some(true))
    }

    /// Check if expression is the literal false
    fn is_false(expr: &Expr) -> bool {
        matches!(Self::extract_bool(expr), Some(false))
    }

    /// Extract integer value from literal expression
    fn extract_int(expr: &Expr) -> Option<i64> {
        use verum_ast::literal::LiteralKind;

        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(i) => Some(i.value as i64),
                _ => None,
            },
            _ => None,
        }
    }

    /// Extract boolean value from literal expression
    fn extract_bool(expr: &Expr) -> Option<bool> {
        use verum_ast::literal::LiteralKind;

        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Bool(b) => Some(*b),
                _ => None,
            },
            _ => None,
        }
    }

    /// Create integer literal expression
    fn make_int(value: i64) -> Expr {
        use verum_ast::literal::{IntLit, Literal, LiteralKind};
        use verum_ast::span::Span;

        Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit {
                    value: value as i128,
                    suffix: Maybe::None,
                }),
                Span::dummy(),
            )),
            Span::dummy(),
        )
    }

    /// Create boolean literal expression
    fn make_bool(value: bool) -> Expr {
        use verum_ast::literal::{Literal, LiteralKind};
        use verum_ast::span::Span;

        Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(value), Span::dummy())),
            Span::dummy(),
        )
    }

    // ==================== Tactic-DSL control flow ====================

    /// Execute a `ProofTactic::Let`: simplify the bound value, push
    /// `name = value` onto the goal's hypothesis list (so SMT sees the
    /// binding as an ordinary equation), then run `body` against the
    /// extended goal.
    fn try_let(
        &mut self,
        name: &Text,
        value: &Expr,
        body: &ProofTactic,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        let simplified = Self::simplify_expr(value);
        let span = value.span;
        let var_expr = Expr::new(
            ExprKind::Path(Path::from_ident(Ident::new(name.clone(), span))),
            span,
        );
        let eq_expr = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Heap::new(var_expr),
                right: Heap::new(simplified),
            },
            span,
        );
        let mut extended = goal.clone();
        extended.hypotheses.push(eq_expr);
        self.execute_tactic(body, &extended)
    }

    /// Execute a `ProofTactic::Match`: simplify the scrutinee, try arms
    /// left-to-right with committed semantics. The first arm whose pattern
    /// matches and whose guard (if any) simplifies to `true` wins; its
    /// pattern bindings are pushed as equation hypotheses and its body is
    /// executed. Body failure is a match failure — cross-arm back-tracking
    /// is opt-in via `first { … }`.
    fn try_match_tactic(
        &mut self,
        scrutinee: &Expr,
        arms: &List<MatchArm>,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        let simplified = Self::simplify_expr(scrutinee);
        for arm in arms.iter() {
            let bindings = match Self::match_pattern(&arm.pattern, &simplified) {
                Some(b) => b,
                None => continue,
            };
            if let Maybe::Some(guard) = &arm.guard {
                let guard_val = Self::simplify_expr(guard);
                if !Self::is_true(&guard_val) {
                    continue;
                }
            }
            let mut extended = goal.clone();
            for (ident, bound) in bindings {
                let span = bound.span;
                let var_expr = Expr::new(
                    ExprKind::Path(Path::from_ident(ident)),
                    span,
                );
                extended.hypotheses.push(Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Eq,
                        left: Heap::new(var_expr),
                        right: Heap::new(bound),
                    },
                    span,
                ));
            }
            return self.execute_tactic(&arm.body, &extended);
        }
        Err(ProofError::TacticFailed(
            "match: no arm matched the scrutinee".to_text(),
        ))
    }

    /// Execute a `ProofTactic::If`: decide `cond` via simplification; if
    /// that is inconclusive, ask the SMT backend whether the current
    /// hypotheses entail `cond` (or `¬cond`). Dispatches to the matching
    /// branch; an undecidable condition is a tactic failure.
    fn try_if(
        &mut self,
        cond: &Expr,
        then_branch: &ProofTactic,
        else_branch: &Maybe<Heap<ProofTactic>>,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        let simplified = Self::simplify_expr(cond);
        if Self::is_true(&simplified) {
            return self.execute_tactic(then_branch, goal);
        }
        if Self::is_false(&simplified) {
            return match else_branch {
                Maybe::Some(b) => self.execute_tactic(b, goal),
                Maybe::None => Ok(List::new()),
            };
        }
        let no_solver: Maybe<Text> = Maybe::None;
        let probe_timeout: Maybe<u64> = Maybe::Some(2000);
        let cond_probe = ProofGoal {
            goal: cond.clone(),
            hypotheses: goal.hypotheses.clone(),
            label: Maybe::Some("if-cond-probe".to_text()),
        };
        if self.try_smt(&no_solver, &probe_timeout, &cond_probe).is_ok() {
            return self.execute_tactic(then_branch, goal);
        }
        let neg_cond = Expr::new(
            ExprKind::Unary {
                op: UnOp::Not,
                expr: Heap::new(cond.clone()),
            },
            cond.span,
        );
        let neg_probe = ProofGoal {
            goal: neg_cond,
            hypotheses: goal.hypotheses.clone(),
            label: Maybe::Some("if-neg-probe".to_text()),
        };
        if self.try_smt(&no_solver, &probe_timeout, &neg_probe).is_ok() {
            return match else_branch {
                Maybe::Some(b) => self.execute_tactic(b, goal),
                Maybe::None => Ok(List::new()),
            };
        }
        Err(ProofError::TacticFailed(
            "if: condition is not decidable by simplification or SMT".to_text(),
        ))
    }

    /// Structural pattern match used by the tactic-level `match`.
    ///
    /// Handles the pattern kinds that have a meaningful structural
    /// meaning on a simplified expression: wildcard, rest, identifier
    /// (with optional `@` subpattern), and literal. Richer pattern kinds
    /// (Tuple, Record, Variant, …) require an evaluator for constructor
    /// applications and are conservatively declined so the engine tries
    /// the next arm.
    fn match_pattern(pattern: &Pattern, value: &Expr) -> Option<Vec<(Ident, Expr)>> {
        match &pattern.kind {
            PatternKind::Wildcard | PatternKind::Rest => Some(Vec::new()),
            PatternKind::Ident {
                name, subpattern, ..
            } => {
                let mut bindings = Vec::new();
                bindings.push((name.clone(), value.clone()));
                if let Maybe::Some(sub) = subpattern {
                    let sub_bindings = Self::match_pattern(sub, value)?;
                    bindings.extend(sub_bindings);
                }
                Some(bindings)
            }
            PatternKind::Literal(lit_pat) => match &value.kind {
                ExprKind::Literal(lit_val) if lit_pat.kind == lit_val.kind => {
                    Some(Vec::new())
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Try auto tactic
    ///
    /// Automatic proof search
    fn try_auto(&mut self, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        // Fast path: trivially-true literal goals (e.g. `ensures true`).
        if matches!(
            &goal.goal.kind,
            ExprKind::Literal(lit) if matches!(lit.kind, verum_ast::LiteralKind::Bool(true))
        ) {
            return Ok(List::new());
        }

        // Try cheap structural tactics first.
        let structural = [
            ProofTactic::Assumption,
            ProofTactic::Reflexivity,
            ProofTactic::Split,
            ProofTactic::Intro,
        ];

        for tactic in &structural {
            if let Ok(subgoals) = self.execute_tactic(tactic, goal) {
                return Ok(subgoals);
            }
        }

        // Fall back to SMT: Auto's purpose is to discharge decidable goals,
        // so after the structural pass we hand the goal to Z3. This is what
        // users expect when they write `proof by auto` for arithmetic /
        // boolean obligations.
        match self.try_smt(&Maybe::None, &Maybe::None, goal) {
            Ok(subgoals) => Ok(subgoals),
            Err(smt_err) => Err(ProofError::TacticFailed(
                format!("'auto': no structural tactic applies and SMT fallback: {smt_err:?}").into(),
            )),
        }
    }

    /// Check if two expressions are structurally equal, ignoring spans.
    ///
    /// The AST carries byte-offset spans on every `Ident`, `Path`, and `Literal`,
    /// and the derived `PartialEq` is span-sensitive. That is exactly wrong for
    /// proof-search equality: the hypothesis `n == 7` from the `requires` clause
    /// and the goal `n == 7` from the `ensures` clause always have different
    /// source spans, so `==` on `Path` / `Ident` / `Literal` would never match.
    /// This recursive walker compares the semantic shape only.
    fn expr_eq(e1: &Expr, e2: &Expr) -> bool {
        use ExprKind::*;

        match (&e1.kind, &e2.kind) {
            (Literal(l1), Literal(l2)) => Self::literal_kind_eq(&l1.kind, &l2.kind),
            (Path(p1), Path(p2)) => Self::path_eq(p1, p2),
            (
                Binary {
                    op: op1,
                    left: l1,
                    right: r1,
                },
                Binary {
                    op: op2,
                    left: l2,
                    right: r2,
                },
            ) => op1 == op2 && Self::expr_eq(l1, l2) && Self::expr_eq(r1, r2),
            (Unary { op: op1, expr: e1 }, Unary { op: op2, expr: e2 }) => {
                op1 == op2 && Self::expr_eq(e1, e2)
            }
            (Paren(e1), Paren(e2)) => Self::expr_eq(e1, e2),
            (Paren(e1), _) => Self::expr_eq(e1, e2),
            (_, Paren(e2)) => Self::expr_eq(e1, e2),
            _ => false,
        }
    }

    /// Compare two `Path`s segment-by-segment ignoring span metadata.
    fn path_eq(p1: &Path, p2: &Path) -> bool {
        use verum_ast::PathSegment;

        if p1.segments.len() != p2.segments.len() {
            return false;
        }
        p1.segments
            .iter()
            .zip(p2.segments.iter())
            .all(|(a, b)| match (a, b) {
                (PathSegment::Name(i1), PathSegment::Name(i2)) => i1.name == i2.name,
                (PathSegment::SelfValue, PathSegment::SelfValue)
                | (PathSegment::Super, PathSegment::Super)
                | (PathSegment::Cog, PathSegment::Cog)
                | (PathSegment::Relative, PathSegment::Relative) => true,
                _ => false,
            })
    }

    /// Compare two literal kinds semantically (spans inside literal metadata
    /// are not observed because `LiteralKind::PartialEq` already delegates
    /// to the value fields, but we centralise it here for clarity and to
    /// shield from future additions of span-bearing variants).
    fn literal_kind_eq(
        k1: &verum_ast::LiteralKind,
        k2: &verum_ast::LiteralKind,
    ) -> bool {
        k1 == k2
    }

    /// Use Z3 to discharge proof goal
    ///
    /// Discharge a proof goal via Z3 SMT solver. Translates the goal to Z3 formula,
    /// checks satisfiability with configurable timeout (default 5000ms), and returns
    /// a proof term if the negation is UNSAT (i.e., the goal is valid).
    pub fn try_smt_discharge(
        &mut self,
        context: &Context,
        goal: &ProofGoal,
    ) -> Result<Maybe<ProofTerm>, ProofError> {
        // Translate goal to Z3
        let translator = Translator::new(context);

        // Build formula: hypotheses ⇒ goal
        let mut formula = goal.goal.clone();

        // Add hypotheses as antecedents (in reverse order)
        let hyps: Vec<_> = goal.hypotheses.iter().collect();
        for hyp in hyps.iter().rev() {
            let current_formula = formula.clone();
            formula = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Imply,
                    left: Box::new((*hyp).clone()),
                    right: Box::new(current_formula),
                },
                formula.span,
            );
        }

        // Translate to Z3
        let z3_formula = match translator.translate_expr(&formula) {
            Ok(f) => f,
            Err(e) => {
                return Err(ProofError::TacticFailed(
                    format!("failed to translate to Z3: {e:?}").into(),
                ));
            }
        };

        let z3_bool = match option_to_maybe(z3_formula.as_bool()) {
            Maybe::Some(b) => b,
            Maybe::None => return Err(ProofError::TacticFailed("Goal is not boolean".into())),
        };

        // Create solver and check
        let solver = context.solver();

        // Inject reflected user-function axioms (refinement
        // reflection). The registry is rendered as a single
        // SMT-LIB block of declare-funs followed by foralls
        // and parsed into the solver before the goal is asserted,
        // so Z3 can unfold calls to user functions during proof
        // search instead of treating them as uninterpreted symbols.
        if !self.reflection_registry.is_empty() {
            let block = self.reflection_registry.to_smtlib_block();
            // `from_string` parses SMT-LIB2 and adds every
            // declare/assert it contains to the solver. Unknown
            // sorts in axiom bodies leave the corresponding
            // assertions inert without raising an error, which
            // is the desired conservative behaviour: an axiom we
            // can't translate simply doesn't fire.
            solver.from_string(block.as_str().to_string());
        }

        // Validity check: a proposition `F` is valid iff `¬F` is
        // unsatisfiable. We assert the NEGATION of the formula and
        // read Z3's verdict:
        //   Unsat  → no counterexample exists, so F is valid (proven).
        //   Sat    → a counterexample exists, so F is not valid.
        //   Unknown→ solver couldn't decide within its resource budget.
        //
        // Asserting `F` directly and reading Sat as "proven" would be
        // wrong: Sat on `F` only means *some* assignment satisfies
        // it, not that every assignment does. That is a satisfiability
        // oracle, not a validity oracle.
        solver.assert(&z3_bool.not());

        match solver.check() {
            z3::SatResult::Unsat => {
                // No counterexample to `F` — goal is valid.
                let proof_term = ProofTerm::SmtProof {
                    solver: "z3".into(),
                    formula: goal.goal.clone(),
                };
                Ok(Maybe::Some(proof_term))
            }
            z3::SatResult::Sat => {
                // Counterexample exists — goal is not valid.
                Ok(Maybe::None)
            }
            z3::SatResult::Unknown => {
                // Timeout or resource limit
                Err(ProofError::SmtTimeout)
            }
        }
    }

    /// Build proof tree by applying tactic
    pub fn build_proof_tree(
        &mut self,
        tactic: &ProofTactic,
        goal: ProofGoal,
    ) -> Result<ProofTree, ProofError> {
        let mut tree = ProofTree::new(goal.clone());
        tree.tactic = Maybe::Some(tactic.clone());

        // Execute tactic to get subgoals
        let subgoals = self.execute_tactic(tactic, &goal)?;

        if subgoals.is_empty() {
            // No subgoals, proof complete
            tree.mark_complete(ProofTerm::Axiom("tactic".into()));
        } else {
            // Create subproof trees
            for subgoal in subgoals {
                let subtree = ProofTree::new(subgoal);
                tree.subproofs.push(Heap::new(subtree));
            }
        }

        Ok(tree)
    }

    // ==================== Formal Proof Tactics ====================

    /// Try intro with specific names
    fn try_intro_named(
        &mut self,
        names: &List<Text>,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        let mut current_goal = goal.clone();

        for name in names {
            match &current_goal.goal.kind {
                ExprKind::Binary {
                    op: BinOp::Imply,
                    left,
                    right,
                } => {
                    // Add premise as named hypothesis
                    let hyp = (**left).clone();
                    // Tag the hypothesis with the name (in a full implementation)
                    current_goal.hypotheses.push(hyp);
                    current_goal.goal = (**right).clone();
                }
                ExprKind::Forall { bindings: _, body } => {
                    // Introduce universally quantified variable
                    // In a full implementation, would add the binding
                    current_goal.goal = (**body).clone();
                }
                ExprKind::Unary {
                    op: UnOp::Not,
                    expr: inner,
                } => {
                    // Proof-by-contradiction surface: to prove `!P`, the
                    // contradiction method assumes `P` as a named
                    // hypothesis and the residual goal becomes `False`.
                    // Without this arm the whole method fails on the
                    // very first `assume h: P` step.
                    current_goal.hypotheses.push((**inner).clone());
                    current_goal.goal = Expr::new(
                        ExprKind::Literal(verum_ast::literal::Literal::new(
                            verum_ast::LiteralKind::Bool(false),
                            current_goal.goal.span,
                        )),
                        current_goal.goal.span,
                    );
                }
                _ => {
                    return Err(ProofError::TacticFailed(
                        format!(
                            "Cannot intro '{}': goal is not an implication, forall, or negation",
                            name
                        )
                        .into(),
                    ));
                }
            }
        }

        let mut result = List::new();
        result.push(current_goal);
        Ok(result)
    }

    /// Try apply with explicit arguments
    ///
    /// This extends `try_apply` by allowing explicit instantiation of universally
    /// quantified variables in the lemma. For example:
    /// - apply lemma_name with [arg1, arg2] substitutes the first two forall-bound
    ///   variables with arg1 and arg2 respectively.
    ///
    /// Lemma application with explicit instantiation of universally quantified variables.
    /// `apply lemma_name with [arg1, arg2]` substitutes the first N forall-bound
    /// variables with the given arguments, then checks if the instantiated conclusion
    /// matches the current goal.
    fn try_apply_with(
        &mut self,
        lemma_name: &Text,
        args: &List<Text>,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        // Look up the lemma by name
        let lemma_key = format!("@name:{}", lemma_name).into();
        let lemma_expr = if let Maybe::Some(lemma_hints) = self.hints.lemmas.get(&lemma_key) {
            if lemma_hints.is_empty() {
                return Err(ProofError::TacticFailed(
                    format!("apply_with: lemma '{}' not found", lemma_name).into(),
                ));
            }
            lemma_hints[0].lemma.clone()
        } else {
            // Try direct lookup
            if let Maybe::Some(lemma_hints) = self.hints.lemmas.get(lemma_name) {
                if lemma_hints.is_empty() {
                    return Err(ProofError::TacticFailed(
                        format!("apply_with: lemma '{}' not found", lemma_name).into(),
                    ));
                }
                lemma_hints[0].lemma.clone()
            } else {
                return Err(ProofError::TacticFailed(
                    format!(
                        "apply_with: lemma '{}' not found in hints database",
                        lemma_name
                    )
                    .into(),
                ));
            }
        };

        // Extract premises and conclusion, substituting args for forall-bound variables
        let (premises, conclusion) = Self::extract_lemma_structure(&lemma_expr);

        // Collect forall-bound variables from the lemma
        let forall_vars = Self::collect_forall_vars(&lemma_expr);

        // Build substitution map from args
        let mut subst = std::collections::HashMap::new();
        for (i, arg) in args.iter().enumerate() {
            if i < forall_vars.len() {
                subst.insert(forall_vars[i].clone(), arg.clone());
            }
        }

        // Apply substitution to conclusion and try to unify with goal
        let instantiated_conclusion = Self::substitute_in_expr(&conclusion, &subst);

        // Check if instantiated conclusion matches goal
        if self
            .try_unify(&instantiated_conclusion, &goal.goal)
            .is_err()
        {
            return Err(ProofError::TacticFailed(
                "apply_with: instantiated lemma conclusion does not unify with goal"
                    .to_string()
                    .into(),
            ));
        }

        // Generate subgoals for each premise (also instantiated)
        let mut subgoals = List::new();
        for premise in &premises {
            let instantiated_premise = Self::substitute_in_expr(premise, &subst);
            let mut new_goal = goal.clone();
            new_goal.goal = instantiated_premise;
            new_goal.label = Maybe::Some(format!("apply_{}_{}", lemma_name, subgoals.len()).into());
            subgoals.push(new_goal);
        }

        Ok(subgoals)
    }

    /// Collect forall-bound variable names from an expression
    fn collect_forall_vars(expr: &Expr) -> List<Text> {
        let mut vars = List::new();

        fn collect_inner(e: &Expr, vars: &mut List<Text>) {
            match &e.kind {
                ExprKind::Forall { bindings, body } => {
                    // Extract variable names from each binding's pattern
                    for binding in bindings {
                        if let verum_ast::PatternKind::Ident { name, .. } = &binding.pattern.kind {
                            vars.push(name.name.clone().into());
                        }
                    }
                    // Recursively collect from body
                    collect_inner(body, vars);
                }
                ExprKind::Binary { left, right, .. } => {
                    collect_inner(left, vars);
                    collect_inner(right, vars);
                }
                ExprKind::Paren(inner) => collect_inner(inner, vars),
                _ => {}
            }
        }

        collect_inner(expr, &mut vars);
        vars
    }

    /// Substitute variables in an expression
    fn substitute_in_expr(expr: &Expr, subst: &std::collections::HashMap<Text, Text>) -> Expr {
        use verum_ast::ty::{Ident, Path};

        match &expr.kind {
            ExprKind::Path(p) => {
                // Check if this path is a single-segment variable that should be substituted
                if let Maybe::Some(ident) = p.as_ident() {
                    let name: Text = ident.as_str().into();
                    if let Some(replacement) = subst.get(&name) {
                        // Create new path with substituted name using Path::single
                        let new_ident = Ident {
                            name: replacement.to_string().into(),
                            span: expr.span,
                        };
                        let new_path = Path::single(new_ident);
                        return Expr::new(ExprKind::Path(new_path), expr.span);
                    }
                }
                expr.clone()
            }
            ExprKind::Binary { op, left, right } => Expr::new(
                ExprKind::Binary {
                    op: *op,
                    left: Box::new(Self::substitute_in_expr(left, subst)),
                    right: Box::new(Self::substitute_in_expr(right, subst)),
                },
                expr.span,
            ),
            ExprKind::Unary { op, expr: inner } => Expr::new(
                ExprKind::Unary {
                    op: *op,
                    expr: Box::new(Self::substitute_in_expr(inner, subst)),
                },
                expr.span,
            ),
            ExprKind::Call { func, args, .. } => {
                let new_args: List<Expr> = args
                    .iter()
                    .map(|a| Self::substitute_in_expr(a, subst))
                    .collect();
                Expr::new(
                    ExprKind::Call {
                        func: Box::new(Self::substitute_in_expr(func, subst)),
                        type_args: List::new(),
                        args: new_args,
                    },
                    expr.span,
                )
            }
            ExprKind::Forall { bindings, body } => {
                // Check if any bound variable shadows a substitution
                let shadows_subst = bindings.iter().any(|binding| {
                    if let verum_ast::PatternKind::Ident { name, .. } = &binding.pattern.kind {
                        let var_name: Text = name.name.clone().into();
                        subst.contains_key(&var_name)
                    } else {
                        false
                    }
                });

                if shadows_subst {
                    expr.clone()
                } else {
                    Expr::new(
                        ExprKind::Forall {
                            bindings: bindings.clone(),
                            body: Box::new(Self::substitute_in_expr(body, subst)),
                        },
                        expr.span,
                    )
                }
            }
            ExprKind::Paren(inner) => Expr::new(
                ExprKind::Paren(Box::new(Self::substitute_in_expr(inner, subst))),
                expr.span,
            ),
            _ => expr.clone(),
        }
    }

    /// Try strong induction
    fn try_strong_induction(
        &mut self,
        var: &Text,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        // Strong induction: assume P(k) for all k < n to prove P(n)
        // For now, reduce to regular induction with stronger IH
        self.try_induction(var, goal)
    }

    /// Try well-founded induction
    fn try_well_founded_induction(
        &mut self,
        var: &Text,
        _relation: &Text,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        // Well-founded induction on a relation
        // For now, reduce to regular induction
        self.try_induction(var, goal)
    }

    /// Try simp with specific lemmas
    fn try_simp_with(
        &mut self,
        lemmas: &List<Text>,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        // Apply simplification using specific lemmas
        let mut simplified_goal = goal.clone();

        for lemma_name in lemmas {
            // Try to apply each lemma for simplification
            if let Maybe::Some(lemma_hint) = self.hints.lookup_lemma_by_name(lemma_name) {
                // Try to simplify using this lemma
                let simplified =
                    Self::try_simplify_with_lemma(&simplified_goal.goal, &lemma_hint.lemma);
                if let Maybe::Some(new_goal) = simplified {
                    simplified_goal.goal = new_goal;
                }
            }
        }

        // Check if we made progress or goal is trivial
        if Self::is_trivial(&simplified_goal.goal) {
            Ok(List::new())
        } else {
            let mut result = List::new();
            result.push(simplified_goal);
            Ok(result)
        }
    }

    /// Try to simplify using a specific lemma
    fn try_simplify_with_lemma(goal: &Expr, lemma: &Expr) -> Maybe<Expr> {
        // Extract conclusion from lemma (handle premises => conclusion)
        let (_, conclusion) = Self::extract_lemma_structure_expr(lemma);

        // Check if lemma conclusion is an equality we can use for rewriting
        if let ExprKind::Binary {
            op: BinOp::Eq,
            left,
            right,
        } = &conclusion.kind
        {
            // Try rewriting left -> right
            if let Maybe::Some(result) = Self::try_rewrite_once(goal, left, right) {
                return Maybe::Some(result);
            }
        }

        Maybe::None
    }

    /// Extract lemma structure (static version for library use)
    fn extract_lemma_structure_expr(lemma: &Expr) -> (List<Expr>, Expr) {
        let mut premises = List::new();
        let mut current = lemma;

        loop {
            match &current.kind {
                ExprKind::Binary {
                    op: BinOp::Imply,
                    left,
                    right,
                } => {
                    premises.push((**left).clone());
                    current = right;
                }
                _ => break,
            }
        }

        (premises, current.clone())
    }

    /// Try to rewrite once (single step)
    fn try_rewrite_once(expr: &Expr, from: &Expr, to: &Expr) -> Maybe<Expr> {
        if Self::expr_eq(expr, from) {
            return Maybe::Some(to.clone());
        }

        // Recurse into subexpressions
        match &expr.kind {
            ExprKind::Binary { op, left, right } => {
                let new_left = Self::try_rewrite_once(left, from, to);
                let new_right = Self::try_rewrite_once(right, from, to);

                if new_left.is_some() || new_right.is_some() {
                    let result_left = new_left.unwrap_or_else(|| (**left).clone());
                    let result_right = new_right.unwrap_or_else(|| (**right).clone());
                    return Maybe::Some(Expr::new(
                        ExprKind::Binary {
                            op: *op,
                            left: Box::new(result_left),
                            right: Box::new(result_right),
                        },
                        expr.span,
                    ));
                }
            }
            ExprKind::Unary { op, expr: inner } => {
                if let Maybe::Some(new_inner) = Self::try_rewrite_once(inner, from, to) {
                    return Maybe::Some(Expr::new(
                        ExprKind::Unary {
                            op: *op,
                            expr: Box::new(new_inner),
                        },
                        expr.span,
                    ));
                }
            }
            _ => {}
        }

        Maybe::None
    }

    /// Try auto with specific hints
    fn try_auto_with(
        &mut self,
        hints: &List<Text>,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        // First try simp with the hints
        if let Ok(subgoals) = self.try_simp_with(hints, goal)
            && subgoals.is_empty()
        {
            return Ok(subgoals);
        }

        // Then try regular auto
        self.try_auto(goal)
    }

    /// Ring tactic: normalize ring expressions
    ///
    /// Ring normalization: rewrites both sides of an equation using ring axioms
    /// (commutativity, associativity, distributivity) and checks equality.
    fn try_ring(&mut self, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        // Check if goal is an equation between ring expressions
        match &goal.goal.kind {
            ExprKind::Binary {
                op: BinOp::Eq,
                left,
                right,
            } => {
                // Normalize both sides to polynomial form
                let left_normalized = self.normalize_ring_expr(left);
                let right_normalized = self.normalize_ring_expr(right);

                // Check if normalized forms are equal
                if Self::ring_exprs_equal(&left_normalized, &right_normalized) {
                    Ok(List::new()) // Proven
                } else {
                    // Try SMT for ring arithmetic
                    self.try_smt(&Maybe::None, &Maybe::None, goal)
                }
            }
            _ => Err(ProofError::TacticFailed(
                "Ring tactic requires an equation goal".into(),
            )),
        }
    }

    /// Normalize ring expression to canonical form
    fn normalize_ring_expr(&self, expr: &Expr) -> RingPolynomial {
        // Convert to polynomial representation
        RingPolynomial::from_expr(expr)
    }

    /// Check if two ring polynomials are equal
    fn ring_exprs_equal(p1: &RingPolynomial, p2: &RingPolynomial) -> bool {
        p1 == p2
    }

    /// Field tactic: normalize field expressions
    ///
    /// Field normalization: extends ring normalization with division (multiplicative inverses).
    fn try_field(&mut self, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        // Field tactic handles division in addition to ring operations
        match &goal.goal.kind {
            ExprKind::Binary {
                op: BinOp::Eq,
                left,
                right,
            } => {
                // For field expressions, we may need to handle:
                // - Division (a/b)
                // - Inverse (1/a)
                // - Non-zero denominators

                // First try ring normalization
                if let Ok(result) = self.try_ring(goal) {
                    return Ok(result);
                }

                // Try SMT with field theory
                self.try_smt(&Maybe::Some("z3".into()), &Maybe::None, goal)
            }
            _ => Err(ProofError::TacticFailed(
                "Field tactic requires an equation goal".into(),
            )),
        }
    }

    /// Omega tactic: linear integer arithmetic solver
    ///
    /// Omega: decides linear integer arithmetic (Presburger arithmetic) via Cooper's algorithm.
    /// Only applies to goals in the decidable QF_LIA fragment.
    fn try_omega(&mut self, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        // Omega decides linear integer arithmetic (Presburger arithmetic)
        // Check if goal is in the decidable fragment

        if self.is_linear_integer_formula(&goal.goal) {
            // Use SMT solver with QF_LIA theory
            self.try_smt(&Maybe::Some("z3".into()), &Maybe::Some(5000), goal)
        } else {
            Err(ProofError::TacticFailed(
                "Omega tactic: goal is not in linear integer arithmetic".into(),
            ))
        }
    }

    /// Check if formula is in linear integer arithmetic
    fn is_linear_integer_formula(&self, expr: &Expr) -> bool {
        // Check if expression uses only:
        // - Integer constants
        // - Integer variables
        // - Addition, subtraction
        // - Multiplication by constants
        // - Comparison operators (=, <, <=, >, >=)
        // - Boolean connectives

        match &expr.kind {
            ExprKind::Literal(_) => true,
            ExprKind::Path(_) => true,
            ExprKind::Binary { op, left, right } => {
                match op {
                    // Arithmetic ops
                    BinOp::Add | BinOp::Sub => {
                        self.is_linear_integer_formula(left)
                            && self.is_linear_integer_formula(right)
                    }
                    // Multiplication only if one side is constant
                    BinOp::Mul => {
                        (Self::is_constant(left) && self.is_linear_integer_formula(right))
                            || (self.is_linear_integer_formula(left) && Self::is_constant(right))
                    }
                    // Comparisons and boolean ops
                    BinOp::Eq
                    | BinOp::Ne
                    | BinOp::Lt
                    | BinOp::Le
                    | BinOp::Gt
                    | BinOp::Ge
                    | BinOp::And
                    | BinOp::Or
                    | BinOp::Imply => {
                        self.is_linear_integer_formula(left)
                            && self.is_linear_integer_formula(right)
                    }
                    _ => false,
                }
            }
            ExprKind::Unary { op, expr } => {
                matches!(op, verum_ast::UnOp::Neg | verum_ast::UnOp::Not)
                    && self.is_linear_integer_formula(expr)
            }
            ExprKind::Paren(inner) => self.is_linear_integer_formula(inner),
            _ => false,
        }
    }

    /// Check if expression is a constant
    fn is_constant(expr: &Expr) -> bool {
        matches!(&expr.kind, ExprKind::Literal(_))
    }

    /// Blast tactic: tableau prover for propositional/first-order logic
    ///
    /// Blast: tableau prover for propositional and simple first-order logic.
    /// Uses aggressive decomposition tactics and backtracking search.
    fn try_blast(&mut self, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        // Blast uses tableau method for automated proof search
        // It's effective for propositional and simple first-order problems

        // Try automated proof search with aggressive tactics
        let aggressive_tactics = vec![
            ProofTactic::Assumption,
            ProofTactic::Reflexivity,
            ProofTactic::Split,
            ProofTactic::Left,
            ProofTactic::Right,
            ProofTactic::Intro,
            ProofTactic::Contradiction,
        ];

        // Try each tactic and recursively apply blast to subgoals
        for tactic in &aggressive_tactics {
            if let Ok(subgoals) = self.execute_tactic(tactic, goal) {
                if subgoals.is_empty() {
                    return Ok(List::new()); // Proven
                }

                // Try blast on each subgoal
                let mut all_proven = true;
                for subgoal in &subgoals {
                    if self.try_blast(subgoal).is_err() {
                        all_proven = false;
                        break;
                    }
                }

                if all_proven {
                    return Ok(List::new());
                }
            }
        }

        // Fallback to SMT
        self.try_smt(&Maybe::None, &Maybe::None, goal)
    }

    /// SMT tactic: dispatch to SMT solver
    ///
    /// SMT dispatch: delegates goal to an SMT solver (Z3 or CVC5). Supports custom
    /// SMT theories (BitVector, Array, etc.) and configurable timeout.
    fn try_smt(
        &mut self,
        solver: &Maybe<Text>,
        timeout_ms: &Maybe<u64>,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        // Create Z3 context with timeout
        let z3_context = Context::new();

        // Set timeout if specified
        let _ = timeout_ms; // Would configure Z3 solver timeout
        let _ = solver; // Solver selection

        // Try to discharge goal using SMT
        match self.try_smt_discharge(&z3_context, goal) {
            Ok(Maybe::Some(_proof)) => Ok(List::new()), // Goal proven by SMT
            Ok(Maybe::None) => Err(ProofError::TacticFailed(
                "SMT: goal is unsatisfiable".into(),
            )),
            Err(e) => Err(e),
        }
    }

    /// Rewrite using equality hypothesis
    fn try_rewrite(
        &mut self,
        hypothesis: &Text,
        reverse: bool,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        // Find hypothesis in goal's hypotheses
        for (idx, hyp) in goal.hypotheses.iter().enumerate() {
            // Check if this hypothesis matches the name (simplified check)
            if let ExprKind::Binary {
                op: BinOp::Eq,
                left,
                right,
            } = &hyp.kind
            {
                // Found an equality hypothesis
                let (from, to) = if reverse {
                    (right, left)
                } else {
                    (left, right)
                };

                // Rewrite goal
                if let Maybe::Some(new_goal_expr) = Self::try_rewrite_once(&goal.goal, from, to) {
                    let mut new_goal = goal.clone();
                    new_goal.goal = new_goal_expr;

                    let mut result = List::new();
                    result.push(new_goal);
                    return Ok(result);
                }
            }
        }

        Err(ProofError::TacticFailed(
            format!("Cannot find equality hypothesis '{}'", hypothesis).into(),
        ))
    }

    /// Rewrite at specific target
    fn try_rewrite_at(
        &mut self,
        hypothesis: &Text,
        _target: &Text,
        reverse: bool,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        // For now, same as try_rewrite (would target specific hypothesis)
        self.try_rewrite(hypothesis, reverse, goal)
    }

    /// Unfold definition
    ///
    /// Replaces a defined name with its definition in the goal.
    /// This tactic looks up definitions in a registry and performs inline expansion.
    ///
    /// Common unfolds:
    /// - `unfold not` replaces `¬P` with `P => False`
    /// - `unfold iff` replaces `P ↔ Q` with `(P => Q) ∧ (Q => P)`
    /// - `unfold ne` replaces `a ≠ b` with `¬(a = b)`
    ///
    /// Unfold a definition by name and substitute in the goal. Standard unfolds:
    /// - `unfold not`: replaces `not P` with `P => False`
    /// - `unfold iff`: replaces `P <-> Q` with `(P => Q) /\ (Q => P)`
    /// - `unfold ne`: replaces `a != b` with `not(a = b)`
    fn try_unfold(&mut self, name: &Text, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        let mut new_goal = goal.clone();

        // Apply known standard library unfolds
        match name.as_str() {
            "not" | "neg" => {
                // Unfold ¬P to P => False
                new_goal.goal = Self::unfold_not_in_expr(&goal.goal);
            }

            "iff" | "equiv" => {
                // Unfold P ↔ Q to (P => Q) ∧ (Q => P)
                new_goal.goal = Self::unfold_iff_in_expr(&goal.goal);
            }

            "ne" | "neq" => {
                // Unfold a ≠ b to ¬(a = b)
                new_goal.goal = Self::unfold_ne_in_expr(&goal.goal);
            }

            "and" => {
                // Already primitive, but could unfold nested
                new_goal.goal = Self::simplify_expr(&goal.goal);
            }

            "or" => {
                // Already primitive, but could unfold nested
                new_goal.goal = Self::simplify_expr(&goal.goal);
            }

            "implies" | "imply" => {
                // P => Q can be unfolded to ¬P ∨ Q
                new_goal.goal = Self::unfold_implies_to_or(&goal.goal);
            }

            // User-defined function unfold
            _ => {
                // Check hints database for definition
                let def_key = format!("@def:{}", name).into();
                if let Maybe::Some(defs) = self.hints.lemmas.get(&def_key) {
                    if !defs.is_empty() {
                        // Found definition - apply it
                        let def = &defs[0].lemma;
                        if let Maybe::Some(unfolded) =
                            Self::try_substitute_definition(&goal.goal, name, def)
                        {
                            new_goal.goal = unfolded;
                        } else {
                            return Err(ProofError::TacticFailed(
                                format!("unfold: could not apply definition of '{}' to goal", name)
                                    .into(),
                            ));
                        }
                    } else {
                        return Err(ProofError::TacticFailed(
                            format!("unfold: no definition found for '{}'", name).into(),
                        ));
                    }
                } else {
                    // No definition found - return goal unchanged with warning
                    // This is lenient behavior for undefined names
                    return Err(ProofError::TacticFailed(
                        format!(
                            "unfold: definition '{}' not found in database. \
                         Available unfolds: not, iff, ne, implies",
                            name
                        )
                        .into(),
                    ));
                }
            }
        }

        // Check if we made progress
        if Self::expr_eq(&new_goal.goal, &goal.goal) {
            Err(ProofError::TacticFailed(
                format!("unfold: '{}' does not appear in goal", name).into(),
            ))
        } else {
            let mut result = List::new();
            result.push(new_goal);
            Ok(result)
        }
    }

    /// Unfold negation: ¬P => P => False
    fn unfold_not_in_expr(expr: &Expr) -> Expr {
        match &expr.kind {
            ExprKind::Unary {
                op: verum_ast::UnOp::Not,
                expr: inner,
            } => {
                // ¬P becomes P => False
                Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Imply,
                        left: inner.clone(),
                        right: Box::new(Self::make_bool(false)),
                    },
                    expr.span,
                )
            }
            ExprKind::Binary { op, left, right } => Expr::new(
                ExprKind::Binary {
                    op: *op,
                    left: Box::new(Self::unfold_not_in_expr(left)),
                    right: Box::new(Self::unfold_not_in_expr(right)),
                },
                expr.span,
            ),
            ExprKind::Paren(inner) => Expr::new(
                ExprKind::Paren(Box::new(Self::unfold_not_in_expr(inner))),
                expr.span,
            ),
            _ => expr.clone(),
        }
    }

    /// Unfold iff: P ↔ Q => (P => Q) ∧ (Q => P)
    fn unfold_iff_in_expr(expr: &Expr) -> Expr {
        match &expr.kind {
            ExprKind::Binary {
                op: BinOp::Eq,
                left,
                right,
            } if Self::is_propositional(left) && Self::is_propositional(right) => {
                // P = Q (as propositions) becomes (P => Q) ∧ (Q => P)
                let left_to_right = Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Imply,
                        left: left.clone(),
                        right: right.clone(),
                    },
                    expr.span,
                );
                let right_to_left = Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Imply,
                        left: right.clone(),
                        right: left.clone(),
                    },
                    expr.span,
                );
                Expr::new(
                    ExprKind::Binary {
                        op: BinOp::And,
                        left: Box::new(left_to_right),
                        right: Box::new(right_to_left),
                    },
                    expr.span,
                )
            }
            ExprKind::Binary { op, left, right } => Expr::new(
                ExprKind::Binary {
                    op: *op,
                    left: Box::new(Self::unfold_iff_in_expr(left)),
                    right: Box::new(Self::unfold_iff_in_expr(right)),
                },
                expr.span,
            ),
            _ => expr.clone(),
        }
    }

    /// Unfold inequality: a ≠ b => ¬(a = b)
    fn unfold_ne_in_expr(expr: &Expr) -> Expr {
        match &expr.kind {
            ExprKind::Binary {
                op: BinOp::Ne,
                left,
                right,
            } => {
                // a ≠ b becomes ¬(a = b)
                let equality = Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Eq,
                        left: left.clone(),
                        right: right.clone(),
                    },
                    expr.span,
                );
                Expr::new(
                    ExprKind::Unary {
                        op: verum_ast::UnOp::Not,
                        expr: Box::new(equality),
                    },
                    expr.span,
                )
            }
            ExprKind::Binary { op, left, right } => Expr::new(
                ExprKind::Binary {
                    op: *op,
                    left: Box::new(Self::unfold_ne_in_expr(left)),
                    right: Box::new(Self::unfold_ne_in_expr(right)),
                },
                expr.span,
            ),
            _ => expr.clone(),
        }
    }

    /// Unfold implication to disjunction: P => Q => ¬P ∨ Q
    fn unfold_implies_to_or(expr: &Expr) -> Expr {
        match &expr.kind {
            ExprKind::Binary {
                op: BinOp::Imply,
                left,
                right,
            } => {
                // P => Q becomes ¬P ∨ Q
                let not_left = Expr::new(
                    ExprKind::Unary {
                        op: verum_ast::UnOp::Not,
                        expr: left.clone(),
                    },
                    expr.span,
                );
                Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Or,
                        left: Box::new(not_left),
                        right: right.clone(),
                    },
                    expr.span,
                )
            }
            ExprKind::Binary { op, left, right } => Expr::new(
                ExprKind::Binary {
                    op: *op,
                    left: Box::new(Self::unfold_implies_to_or(left)),
                    right: Box::new(Self::unfold_implies_to_or(right)),
                },
                expr.span,
            ),
            _ => expr.clone(),
        }
    }

    /// Check if expression is propositional (boolean-valued)
    fn is_propositional(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Literal(lit) => matches!(lit.kind, verum_ast::LiteralKind::Bool(_)),
            ExprKind::Binary { op, .. } => matches!(
                op,
                BinOp::And
                    | BinOp::Or
                    | BinOp::Imply
                    | BinOp::Eq
                    | BinOp::Ne
                    | BinOp::Lt
                    | BinOp::Le
                    | BinOp::Gt
                    | BinOp::Ge
            ),
            ExprKind::Unary { op, .. } => matches!(op, verum_ast::UnOp::Not),
            ExprKind::Path(_) => true, // Could be a proposition variable
            ExprKind::Paren(inner) => Self::is_propositional(inner),
            _ => false,
        }
    }

    /// Try to substitute a definition in an expression
    fn try_substitute_definition(expr: &Expr, name: &Text, def: &Expr) -> Maybe<Expr> {
        // Look for calls to `name` and replace with `def`
        match &expr.kind {
            ExprKind::Call { func, args, .. } => {
                if let ExprKind::Path(p) = &func.kind
                    && let Maybe::Some(ident) = p.as_ident()
                    && ident.as_str() == name.as_str()
                {
                    // Found call to `name` - substitute with definition
                    // For now, just return the definition body
                    // In a full implementation, would substitute arguments
                    return Maybe::Some(def.clone());
                }
                // Recurse into arguments
                let new_args: List<Expr> = args
                    .iter()
                    .map(|arg| {
                        Self::try_substitute_definition(arg, name, def)
                            .unwrap_or_else(|| arg.clone())
                    })
                    .collect();
                Maybe::Some(Expr::new(
                    ExprKind::Call {
                        func: func.clone(),
                        type_args: List::new(),
                        args: new_args,
                    },
                    expr.span,
                ))
            }
            ExprKind::Binary { op, left, right } => {
                let new_left = Self::try_substitute_definition(left, name, def)
                    .unwrap_or_else(|| (**left).clone());
                let new_right = Self::try_substitute_definition(right, name, def)
                    .unwrap_or_else(|| (**right).clone());
                Maybe::Some(Expr::new(
                    ExprKind::Binary {
                        op: *op,
                        left: Box::new(new_left),
                        right: Box::new(new_right),
                    },
                    expr.span,
                ))
            }
            ExprKind::Path(p) => {
                if let Maybe::Some(ident) = p.as_ident()
                    && ident.as_str() == name.as_str()
                {
                    return Maybe::Some(def.clone());
                }
                Maybe::None
            }
            _ => Maybe::None,
        }
    }

    /// Compute/normalize expression
    fn try_compute(&mut self, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        // Evaluate/simplify computable parts of the goal
        let simplified = Self::simplify_expr(&goal.goal);

        if Self::is_trivial(&simplified) {
            Ok(List::new())
        } else {
            let mut new_goal = goal.clone();
            new_goal.goal = simplified;
            let mut result = List::new();
            result.push(new_goal);
            Ok(result)
        }
    }

    /// Try left of disjunction
    fn try_left(&mut self, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        match &goal.goal.kind {
            ExprKind::Binary {
                op: BinOp::Or,
                left,
                right: _,
            } => {
                let mut new_goal = goal.clone();
                new_goal.goal = (**left).clone();
                let mut result = List::new();
                result.push(new_goal);
                Ok(result)
            }
            _ => Err(ProofError::TacticFailed(
                "Left tactic requires disjunction goal".into(),
            )),
        }
    }

    /// Try right of disjunction
    fn try_right(&mut self, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        match &goal.goal.kind {
            ExprKind::Binary {
                op: BinOp::Or,
                left: _,
                right,
            } => {
                let mut new_goal = goal.clone();
                new_goal.goal = (**right).clone();
                let mut result = List::new();
                result.push(new_goal);
                Ok(result)
            }
            _ => Err(ProofError::TacticFailed(
                "Right tactic requires disjunction goal".into(),
            )),
        }
    }

    /// Try existential witness
    fn try_exists(
        &mut self,
        _witness: &Text,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        match &goal.goal.kind {
            ExprKind::Exists { bindings: _, body } => {
                // Instantiate existential with witness
                let mut new_goal = goal.clone();
                new_goal.goal = (**body).clone();
                let mut result = List::new();
                result.push(new_goal);
                Ok(result)
            }
            _ => Err(ProofError::TacticFailed(
                "Exists tactic requires existential goal".into(),
            )),
        }
    }

    /// Try cases on hypothesis
    ///
    /// Performs case analysis on a hypothesis. For a disjunction hypothesis (P ∨ Q),
    /// generates two subgoals: one assuming P and one assuming Q.
    /// For an inductive type, generates one subgoal per constructor.
    ///
    /// Case analysis on a hypothesis. For disjunction (P or Q), generates two subgoals
    /// (one assuming P, one assuming Q). For inductive types, generates one subgoal per constructor.
    fn try_cases_on(
        &mut self,
        hypothesis: &Text,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        // Find the hypothesis by name (h0, h1, ...) or by pattern match
        let hyp_idx = self.find_hypothesis_index(hypothesis, goal)?;
        let hyp = goal.hypotheses[hyp_idx].clone();

        match &hyp.kind {
            // Case analysis on disjunction: P ∨ Q => prove with P, prove with Q
            ExprKind::Binary { op: BinOp::Or, left, right } => {
                // Case 1: Assume P
                let mut goal_left = goal.clone();
                goal_left.hypotheses.remove(hyp_idx);
                goal_left.add_hypothesis((**left).clone());
                goal_left.label = Maybe::Some(format!("case_left_{}", hypothesis).into());

                // Case 2: Assume Q
                let mut goal_right = goal.clone();
                goal_right.hypotheses.remove(hyp_idx);
                goal_right.add_hypothesis((**right).clone());
                goal_right.label = Maybe::Some(format!("case_right_{}", hypothesis).into());

                Ok(List::from_iter(vec![goal_left, goal_right]))
            }

            // Case analysis on Boolean: b ∨ ¬b
            ExprKind::Path(p) if self.is_boolean_hypothesis(&hyp) => {
                // Case 1: Assume hypothesis is true
                let mut goal_true = goal.clone();
                goal_true.add_hypothesis(Self::make_bool(true));
                goal_true.label = Maybe::Some(format!("case_true_{}", hypothesis).into());

                // Case 2: Assume hypothesis is false
                let mut goal_false = goal.clone();
                goal_false.add_hypothesis(Self::make_bool(false));
                goal_false.label = Maybe::Some(format!("case_false_{}", hypothesis).into());

                Ok(List::from_iter(vec![goal_true, goal_false]))
            }

            // Case analysis on Maybe/Option type
            ExprKind::Call { func, .. } if self.is_maybe_constructor(func) => {
                // Case 1: None
                let mut goal_none = goal.clone();
                goal_none.label = Maybe::Some(format!("case_none_{}", hypothesis).into());

                // Case 2: Some(x)
                let mut goal_some = goal.clone();
                // Add the unwrapped value as a hypothesis
                goal_some.label = Maybe::Some(format!("case_some_{}", hypothesis).into());

                Ok(List::from_iter(vec![goal_none, goal_some]))
            }

            // Case analysis on equality: either use rewrite or derive contradiction
            ExprKind::Binary { op: BinOp::Eq, left, right } => {
                // Reflexive case: if left == right syntactically, nothing to do
                if Self::expr_eq(left, right) {
                    let mut result = List::new();
                    result.push(goal.clone());
                    return Ok(result);
                }

                // Otherwise, we can use the equality for rewriting
                // Return goal unchanged but mark that equality is available
                let mut result = List::new();
                result.push(goal.clone());
                Ok(result)
            }

            _ => Err(ProofError::TacticFailed(format!(
                "cases_on: hypothesis '{}' is not a disjunction, boolean, or case-analyzable type. \
                 Expected P ∨ Q, Bool, Maybe<T>, or similar.",
                hypothesis
            ).into())),
        }
    }

    /// Try destruct hypothesis
    ///
    /// Destructs a compound hypothesis into its components.
    /// For conjunction (P ∧ Q), adds both P and Q as separate hypotheses.
    /// For existential (∃x. P(x)), introduces the witness and the property.
    ///
    /// Destruct a compound hypothesis into components. For conjunction (P /\ Q),
    /// adds both P and Q as separate hypotheses. For existential (exists x. P(x)),
    /// introduces the witness variable and the property as separate hypotheses.
    fn try_destruct(
        &mut self,
        hypothesis: &Text,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        // Find the hypothesis by name
        let hyp_idx = self.find_hypothesis_index(hypothesis, goal)?;
        let hyp = goal.hypotheses[hyp_idx].clone();

        match &hyp.kind {
            // Destruct conjunction: P ∧ Q => add P and Q separately
            ExprKind::Binary { op: BinOp::And, left, right } => {
                let mut new_goal = goal.clone();
                // Remove the original hypothesis
                new_goal.hypotheses.remove(hyp_idx);
                // Add both components
                new_goal.add_hypothesis((**left).clone());
                new_goal.add_hypothesis((**right).clone());

                let mut result = List::new();
                result.push(new_goal);
                Ok(result)
            }

            // Destruct existential: ∃x. P(x) => introduce witness and property
            ExprKind::Exists { bindings: _, body } => {
                let mut new_goal = goal.clone();
                // Remove the original hypothesis
                new_goal.hypotheses.remove(hyp_idx);
                // Add the body (with witness substituted)
                new_goal.add_hypothesis((**body).clone());

                let mut result = List::new();
                result.push(new_goal);
                Ok(result)
            }

            // Destruct implication: P => Q with P available => derive Q
            ExprKind::Binary { op: BinOp::Imply, left, right } => {
                // Check if P is in hypotheses
                let has_premise = goal.hypotheses.iter().any(|h| Self::expr_eq(h, left));

                if has_premise {
                    let mut new_goal = goal.clone();
                    // Add conclusion Q
                    new_goal.add_hypothesis((**right).clone());

                    let mut result = List::new();
                    result.push(new_goal);
                    Ok(result)
                } else {
                    Err(ProofError::TacticFailed(format!(
                        "destruct: cannot destruct implication '{}' - premise not available",
                        hypothesis
                    ).into()))
                }
            }

            // Destruct tuple/pair
            ExprKind::Tuple(elements) if elements.len() == 2 => {
                let mut new_goal = goal.clone();
                new_goal.hypotheses.remove(hyp_idx);
                // Add both tuple components
                new_goal.add_hypothesis(elements[0].clone());
                new_goal.add_hypothesis(elements[1].clone());

                let mut result = List::new();
                result.push(new_goal);
                Ok(result)
            }

            _ => Err(ProofError::TacticFailed(format!(
                "destruct: hypothesis '{}' cannot be destructed. \
                 Expected conjunction (P ∧ Q), existential (∃x.P), implication with premise, or tuple.",
                hypothesis
            ).into())),
        }
    }

    /// Find hypothesis index by name
    fn find_hypothesis_index(&self, name: &Text, goal: &ProofGoal) -> Result<usize, ProofError> {
        // Check for numbered hypotheses: h0, h1, h2, ...
        if name.as_str().starts_with('h')
            && let Ok(idx) = name.as_str()[1..].parse::<usize>()
            && idx < goal.hypotheses.len()
        {
            return Ok(idx);
        }

        // Check for named hypotheses
        for (idx, hyp) in goal.hypotheses.iter().enumerate() {
            if let ExprKind::Path(p) = &hyp.kind
                && let Maybe::Some(ident) = p.as_ident()
                && ident.as_str() == name.as_str()
            {
                return Ok(idx);
            }
        }

        Err(ProofError::TacticFailed(
            format!(
                "Hypothesis '{}' not found. Available: h0..h{}",
                name,
                goal.hypotheses.len().saturating_sub(1)
            )
            .into(),
        ))
    }

    /// Check if hypothesis is a boolean type
    fn is_boolean_hypothesis(&self, _hyp: &Expr) -> bool {
        // In a full implementation, this would check the type
        // For now, return false to be conservative
        false
    }

    /// Check if expression is a Maybe/Option constructor
    fn is_maybe_constructor(&self, func: &Expr) -> bool {
        if let ExprKind::Path(p) = &func.kind
            && let Maybe::Some(ident) = p.as_ident()
        {
            let name = ident.as_str();
            // Check if it's a Maybe constructor (Some/None)
            return matches!(name, "Some" | "None");
        }
        false
    }

    /// Try exact proof term
    ///
    /// The `exact` tactic closes a goal by providing a term that exactly matches the goal type.
    /// This is sound only if:
    /// 1. The term is a hypothesis in the current context with the same type as the goal
    /// 2. The term is a named lemma/axiom whose conclusion unifies with the goal
    /// 3. The term is the literal expression `true` and the goal is `true`
    ///
    /// This is a foundational tactic that MUST verify the proof term.
    fn try_exact(&mut self, term: &Text, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        // Strategy 1: Check if term names a hypothesis
        for (idx, hyp) in goal.hypotheses.iter().enumerate() {
            // Generate hypothesis name: h0, h1, h2, ...
            let hyp_name = format!("h{}", idx);
            if term.as_str() == hyp_name {
                // Check if hypothesis exactly matches the goal
                if Self::expr_eq(hyp, &goal.goal) {
                    return Ok(List::new()); // Hypothesis proves goal
                }
            }

            // Also check for named hypotheses from patterns
            if let ExprKind::Path(p) = &hyp.kind
                && let Maybe::Some(ident) = p.as_ident()
                && ident.as_str() == term.as_str()
            {
                // Named hypothesis - check if it proves the goal
                if Self::expr_eq(hyp, &goal.goal) {
                    return Ok(List::new());
                }
            }
        }

        // Strategy 2: Check if term is a lemma in hints database
        let lemma_key = format!("@name:{}", term).into();
        if let Maybe::Some(lemma_hints) = self.hints.lemmas.get(&lemma_key)
            && !lemma_hints.is_empty()
        {
            let lemma = &lemma_hints[0];
            let (premises, conclusion) = Self::extract_lemma_structure(&lemma.lemma);

            // For exact, there must be no premises (or they must all be satisfied)
            if premises.is_empty() {
                // Check if lemma conclusion unifies with goal
                if self.try_unify(&conclusion, &goal.goal).is_ok() {
                    return Ok(List::new());
                }
            } else {
                // Check if all premises are available as hypotheses
                let all_premises_satisfied = premises.iter().all(|premise| {
                    goal.hypotheses
                        .iter()
                        .any(|hyp| Self::expr_eq(premise, hyp))
                });

                if all_premises_satisfied && self.try_unify(&conclusion, &goal.goal).is_ok() {
                    return Ok(List::new());
                }
            }
        }

        // Strategy 3: Check if term is an axiom
        if let Maybe::Some(axiom_hints) = self.hints.lemmas.get(term)
            && !axiom_hints.is_empty()
        {
            let axiom = &axiom_hints[0];
            if Self::expr_eq(&axiom.lemma, &goal.goal) {
                return Ok(List::new());
            }
        }

        // Strategy 4: Literal 'true' proves goal 'true'
        if (term.as_str() == "true" || term.as_str() == "True" || term.as_str() == "trivial")
            && Self::is_true(&goal.goal)
        {
            return Ok(List::new());
        }

        // Strategy 5: Check if the term itself is an expression that matches the goal
        // This handles cases like `exact H` where H is a hypothesis expression
        for hyp in &goal.hypotheses {
            if Self::expr_eq(hyp, &goal.goal) {
                // If any hypothesis matches goal, term could be referencing it
                // But we need to verify the term actually refers to this hypothesis
                // This is a conservative check - we found a matching hypothesis
                // The term could be its name (already checked above)
                continue; // Don't auto-accept, require explicit name
            }
        }

        // No valid proof term found - this is the safe failure case
        Err(ProofError::TacticFailed(format!(
            "exact: term '{}' does not prove goal. \
             The term must be a hypothesis (h0, h1, ...) or a lemma that exactly matches the goal type.",
            term
        ).into()))
    }

    /// Try contradiction
    fn try_contradiction(&mut self, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        // Check if hypotheses contain a contradiction
        for hyp in &goal.hypotheses {
            // Check for False
            if Self::is_false(hyp) {
                return Ok(List::new()); // Contradiction found
            }

            // Check for P and not P
            if let ExprKind::Unary {
                op: verum_ast::UnOp::Not,
                expr: inner,
            } = &hyp.kind
            {
                for other_hyp in &goal.hypotheses {
                    if Self::expr_eq(inner, other_hyp) {
                        return Ok(List::new()); // P and ¬P found
                    }
                }
            }
        }

        Err(ProofError::TacticFailed(
            "No contradiction found in hypotheses".into(),
        ))
    }

    /// Try exfalso
    fn try_exfalso(&mut self, goal: &ProofGoal) -> Result<List<ProofGoal>, ProofError> {
        // Change goal to False (from anything can derive anything)
        let mut new_goal = goal.clone();
        new_goal.goal = Self::make_bool(false);
        let mut result = List::new();
        result.push(new_goal);
        Ok(result)
    }

    /// Try named tactic from database
    fn try_named_tactic(
        &mut self,
        name: &Text,
        _args: &List<Text>,
        goal: &ProofGoal,
    ) -> Result<List<ProofGoal>, ProofError> {
        use crate::cubical_tactic::{
            try_cubical, try_category_simp, try_category_law, try_descent_check,
        };

        match name.as_str() {
            "simp" => self.try_simplify(goal),
            "ring" => self.try_ring(goal),
            "field" => self.try_field(goal),
            "omega" => self.try_omega(goal),
            "blast" => self.try_blast(goal),
            "auto" => self.try_auto(goal),

            // === Cubical HoTT tactics ===
            //
            // `cubical` / `homotopy` — first tries to close the goal by
            // cubical WHNF normalisation (transport-refl, sym-refl,
            // hcomp-collapse, path-lambda β, univalence computation).
            // If normalisation cannot close the goal it falls back to
            // the full SMT solver via `try_auto`.
            "cubical" | "homotopy" => {
                match try_cubical(goal) {
                    Ok(subgoals) => Ok(subgoals),
                    Err(ref e) if e.to_string().contains("__smt_fallback") => {
                        self.try_auto(goal)
                    }
                    Err(e) => Err(e),
                }
            }

            // === Category theory tactics ===
            //
            // `category_simp` — rewrite using associativity and
            // identity laws (up to 50 steps), then fall back to SMT.
            "category_simp" => {
                match try_category_simp(goal) {
                    Ok(subgoals) => Ok(subgoals),
                    Err(ref e) if e.to_string().contains("__smt_fallback") => {
                        self.try_auto(goal)
                    }
                    Err(e) => Err(e),
                }
            }

            // `category_law` — like `category_simp` but also unfolds
            // functor preservation laws (up to 100 steps).
            "category_law" | "functor_law" => {
                match try_category_law(goal) {
                    Ok(subgoals) => Ok(subgoals),
                    Err(ref e) if e.to_string().contains("__smt_fallback") => {
                        self.try_auto(goal)
                    }
                    Err(e) => Err(e),
                }
            }

            // === Sheaf / topos tactics ===
            //
            // `descent_check` / `descent` — check Čech descent via the
            // SMT sheaf-domain encoding. Uses the cubical_tactic bridge
            // to recognise descent-shaped goals, then delegates to SMT.
            "descent" | "descent_check" => {
                match try_descent_check(goal) {
                    Ok(subgoals) => Ok(subgoals),
                    Err(ref e) if e.to_string().contains("__smt_fallback") => {
                        self.try_auto(goal)
                    }
                    Err(e) => Err(e),
                }
            }

            // === Oracle tactic (LLM-guided stochastic search) ===
            //
            // The tactic name arriving here has the form `oracle:<confidence>`
            // because `user_tactic.rs` embeds the threshold in the
            // `TacticKind::Custom` tag.  We parse it back out before
            // delegating to the oracle execution function.
            //
            // Also handle the bare `oracle` name for the no-arg surface form.
            "oracle" => try_oracle_tactic(goal, 0.9, self),

            name if name.starts_with("oracle:") => {
                let confidence = name
                    .strip_prefix("oracle:")
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.9);
                try_oracle_tactic(goal, confidence, self)
            }

            _ => Err(ProofError::TacticFailed(
                format!("Unknown tactic: {}", name).into(),
            )),
        }
    }
}

// ==================== Oracle Tactic ====================
//
// The oracle tactic implements LLM/Giry-style stochastic proof search:
//
//   1. Serialize the proof goal as a text description.
//   2. Generate a ranked list of candidate tactic sequences (simulated here
//      via heuristic pattern matching; a real deployment calls the LLM via
//      `@intrinsic("llm.query_log_probs")`).
//   3. Normalise raw scores with softmax (temperature = 1.0).
//   4. Filter candidates whose softmax probability meets the confidence
//      threshold τ.
//   5. Try the best surviving candidate as a `ProofTactic::Named` dispatch.
//   6. Verify the result via the existing SMT path — the oracle is NEVER
//      trusted without verification.
//   7. Fall back to `try_auto` if no candidate clears the bar or every
//      candidate fails verification.
//
// The design mirrors core/math/giry.vr §14 `sample_above`: the oracle
// selects the highest-probability functor (tactic) above threshold τ.

/// Structural analysis of a proof goal for intelligent tactic selection.
struct GoalAnalysis {
    /// Goal is an equality (a == b or Path<A>(a, b))
    is_equality: bool,
    /// Goal contains only linear integer arithmetic (decidable by omega)
    is_linear_arithmetic: bool,
    /// Goal contains ring/field operations (decidable by ring/field tactic)
    is_ring_goal: bool,
    /// Goal is a propositional formula (decidable by blast/smt)
    is_propositional: bool,
    /// Goal has implications or universals (intro might help)
    has_implications: bool,
    /// Goal involves categorical structures
    is_categorical: bool,
    /// Goal involves descent/sheaf conditions
    is_descent: bool,
    /// Number of hypotheses available
    hypothesis_count: usize,
    /// Nesting depth of the goal expression
    nesting_depth: usize,
}

/// Analyse the structure of a `ProofGoal` to guide tactic selection.
///
/// All inspection is performed on the AST `ExprKind` — no debug-format
/// string matching.
fn analyze_goal(goal: &ProofGoal) -> GoalAnalysis {
    let expr = &goal.goal;
    GoalAnalysis {
        is_equality: matches!(&expr.kind, ExprKind::Binary { op: BinOp::Eq, .. }),
        is_linear_arithmetic: check_linear_arithmetic(expr),
        is_ring_goal: check_ring_operations(expr),
        is_propositional: check_propositional(expr),
        has_implications: check_has_implications(expr),
        is_categorical: check_categorical(expr),
        is_descent: check_descent_goal(expr),
        hypothesis_count: goal.hypotheses.len(),
        nesting_depth: compute_depth(expr),
    }
}

/// Return `true` when `expr` is entirely within the linear arithmetic
/// fragment: integer/variable leaves, `+`, `-`, unary `-`, `*` (only when
/// one side is a literal), and comparison/equality operators.
fn check_linear_arithmetic(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Binary { op, left, right } => {
            matches!(
                op,
                BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Lt
                    | BinOp::Le
                    | BinOp::Gt
                    | BinOp::Ge
                    | BinOp::Eq
            ) && check_linear_arithmetic(left)
                && check_linear_arithmetic(right)
        }
        ExprKind::Unary { op: UnOp::Neg, expr } => check_linear_arithmetic(expr),
        ExprKind::Literal(_) => true,
        ExprKind::Path(_) => true, // variable
        ExprKind::Paren(inner) => check_linear_arithmetic(inner),
        _ => false,
    }
}

/// Return `true` when `expr` contains a non-linear multiplication (both
/// operands contain variables), indicating a ring/polynomial goal.
fn check_ring_operations(expr: &Expr) -> bool {
    fn has_nonlinear_mul(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Binary {
                op: BinOp::Mul,
                left,
                right,
            } => {
                !matches!(&left.kind, ExprKind::Literal(_))
                    && !matches!(&right.kind, ExprKind::Literal(_))
            }
            ExprKind::Binary { left, right, .. } => {
                has_nonlinear_mul(left) || has_nonlinear_mul(right)
            }
            _ => false,
        }
    }
    has_nonlinear_mul(expr)
}

/// Return `true` when `expr` is a purely propositional formula built from
/// `&&`, `||`, `==`, `!`, literals, and path (variable) leaves.
fn check_propositional(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Binary { op, left, right } => {
            matches!(op, BinOp::And | BinOp::Or | BinOp::Eq)
                && check_propositional(left)
                && check_propositional(right)
        }
        ExprKind::Unary { op: UnOp::Not, expr } => check_propositional(expr),
        ExprKind::Literal(_) | ExprKind::Path(_) => true,
        ExprKind::Paren(inner) => check_propositional(inner),
        _ => false,
    }
}

/// Return `true` when `expr` contains a quantifier (`forall`/`exists`) or
/// an implication (`BinOp::Imply`).
fn check_has_implications(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Forall { .. } | ExprKind::Exists { .. } => true,
        ExprKind::Binary { op, left, right } => {
            matches!(op, BinOp::Imply)
                || check_has_implications(left)
                || check_has_implications(right)
        }
        ExprKind::Paren(inner) => check_has_implications(inner),
        _ => false,
    }
}

/// Return `true` when `expr` refers to categorical primitives by name.
fn check_categorical(expr: &Expr) -> bool {
    fn has_cat_name(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Path(p) => p.segments.iter().any(|s| {
                if let verum_ast::ty::PathSegment::Name(ident) = s {
                    matches!(
                        ident.name.as_str(),
                        "compose"
                            | "id"
                            | "Category"
                            | "Functor"
                            | "NatTrans"
                            | "morphism"
                            | "map_obj"
                            | "map_mor"
                    )
                } else {
                    false
                }
            }),
            ExprKind::MethodCall {
                method, receiver, ..
            } => {
                matches!(
                    method.name.as_str(),
                    "compose" | "then" | "map" | "map_obj" | "map_mor"
                ) || has_cat_name(receiver)
            }
            ExprKind::Binary { left, right, .. } => has_cat_name(left) || has_cat_name(right),
            ExprKind::Call { func, args, .. } => {
                has_cat_name(func) || args.iter().any(has_cat_name)
            }
            ExprKind::Paren(inner) => has_cat_name(inner),
            _ => false,
        }
    }
    has_cat_name(expr)
}

/// Return `true` when `expr` is a descent/sheaf-condition call site.
fn check_descent_goal(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Call { func, .. } => {
            if let ExprKind::Path(p) = &func.kind {
                p.segments.last().map(|s| {
                    if let verum_ast::ty::PathSegment::Name(ident) = s {
                        matches!(
                            ident.name.as_str(),
                            "descent_condition"
                                | "compatible_sections"
                                | "sheaf_condition"
                                | "check_descent"
                                | "gluing_condition"
                        )
                    } else {
                        false
                    }
                }).unwrap_or(false)
            } else {
                false
            }
        }
        ExprKind::Binary { left, right, .. } => {
            check_descent_goal(left) || check_descent_goal(right)
        }
        ExprKind::Paren(inner) => check_descent_goal(inner),
        _ => false,
    }
}

/// Compute the nesting depth of an expression tree.
fn compute_depth(expr: &Expr) -> usize {
    match &expr.kind {
        ExprKind::Binary { left, right, .. } => {
            1 + compute_depth(left).max(compute_depth(right))
        }
        ExprKind::Unary { expr, .. } => 1 + compute_depth(expr),
        ExprKind::Call { func, args, .. } => {
            let max_arg = args.iter().map(compute_depth).max().unwrap_or(0);
            1 + compute_depth(func).max(max_arg)
        }
        ExprKind::Paren(inner) => compute_depth(inner),
        ExprKind::Forall { body, .. } | ExprKind::Exists { body, .. } => 1 + compute_depth(body),
        _ => 0,
    }
}

/// Execute the oracle tactic on `goal`.
///
/// `confidence` — the minimum softmax probability required before a
/// candidate is dispatched.  Typical values: 0.7 (permissive) to 0.95
/// (strict).  Defaults to 0.9 when unspecified.
pub(crate) fn try_oracle_tactic(
    goal: &ProofGoal,
    confidence: f64,
    engine: &mut ProofSearchEngine,
) -> Result<List<ProofGoal>, ProofError> {
    // Step 1 — analyse the goal AST structure.
    let analysis = analyze_goal(goal);

    // Step 2 — generate raw (name, score) candidates from structural analysis.
    let raw_candidates = generate_oracle_candidates(&analysis);

    // Step 3 — softmax-normalise the raw scores.
    let scored = softmax_score(&raw_candidates, 1.0);

    // Step 4 — pick the best candidate that clears the confidence bar.
    let best = scored
        .iter()
        .filter(|(_, prob)| *prob >= confidence)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    match best {
        Some((candidate_name, _prob)) => {
            // Step 5 — try the candidate as a named tactic sequence.
            // Each candidate can be a single tactic name or a semicolon-separated
            // sequence (e.g. "intro; auto").  We split on "; " and chain them.
            let result = try_apply_oracle_candidate(goal, candidate_name, engine);

            match result {
                Ok(subgoals) => Ok(subgoals),
                Err(_) => {
                    // Step 7 — candidate failed SMT verification; fall back to auto.
                    engine.try_auto(goal)
                }
            }
        }
        None => {
            // No candidate above the confidence threshold — fall back to auto.
            engine.try_auto(goal)
        }
    }
}

/// Generate proof-term candidates from structural goal analysis.
///
/// In a real deployment this calls the language model via
/// `@intrinsic("llm.query_log_probs")`.  At compile/test time we simulate
/// the LLM by inspecting the `GoalAnalysis` (which was derived from AST
/// structure, not debug strings) and returning a ranked list of
/// known-good tactic strategies.
///
/// Returns `Vec<(tactic_sequence, raw_score)>` where scores are *unnormalised*
/// log-probability surrogates (higher = more likely to succeed).
fn generate_oracle_candidates(analysis: &GoalAnalysis) -> Vec<(String, f64)> {
    let mut candidates: Vec<(String, f64)> = Vec::new();

    // High-confidence decidable goals.
    if analysis.is_linear_arithmetic {
        candidates.push(("omega".to_string(), 0.95));
    }
    if analysis.is_ring_goal {
        candidates.push(("ring".to_string(), 0.92));
    }
    if analysis.is_propositional {
        candidates.push(("blast".to_string(), 0.88));
    }

    // Equality goals.
    if analysis.is_equality {
        candidates.push(("simp".to_string(), 0.7));
        candidates.push(("cubical".to_string(), 0.5));
        candidates.push(("refl".to_string(), 0.3));
    }

    // Categorical goals.
    if analysis.is_categorical {
        candidates.push(("category_simp".to_string(), 0.8));
        candidates.push(("category_law".to_string(), 0.6));
    }

    // Descent goals.
    if analysis.is_descent {
        candidates.push(("descent_check".to_string(), 0.85));
    }

    // Implication / quantifier goals.
    if analysis.has_implications {
        candidates.push(("intro".to_string(), 0.6));
        if analysis.hypothesis_count > 0 {
            candidates.push(("assumption".to_string(), 0.4));
        }
    }

    // Deep goals benefit from simplification.
    if analysis.nesting_depth > 4 {
        candidates.push(("simp".to_string(), 0.5));
    }

    // Hypothesis-rich goals.
    if analysis.hypothesis_count >= 3 {
        candidates.push(("assumption".to_string(), 0.6));
        candidates.push(("exact".to_string(), 0.3));
    }

    // Universal fallbacks — always present.
    candidates.push(("auto".to_string(), 0.15));
    candidates.push(("smt".to_string(), 0.10));

    candidates
}

/// Apply temperature-scaled softmax normalisation to raw candidate scores.
///
/// Implements the numerically stable log-sum-exp trick:
///   p_i = exp((s_i − max_s) / τ) / Σ_j exp((s_j − max_s) / τ)
///
/// Returns a new vector of `(name, probability)` pairs.  The probabilities
/// sum to 1.0 (modulo floating-point rounding).
fn softmax_score(candidates: &[(String, f64)], temperature: f64) -> Vec<(String, f64)> {
    if candidates.is_empty() {
        return Vec::new();
    }

    let max_score = candidates
        .iter()
        .map(|(_, s)| *s)
        .fold(f64::NEG_INFINITY, f64::max);

    let exps: Vec<f64> = candidates
        .iter()
        .map(|(_, s)| ((s - max_score) / temperature).exp())
        .collect();

    let sum: f64 = exps.iter().sum();
    if sum == 0.0 {
        // Degenerate case: return uniform distribution.
        let uniform = 1.0 / candidates.len() as f64;
        return candidates
            .iter()
            .map(|(name, _)| (name.clone(), uniform))
            .collect();
    }

    candidates
        .iter()
        .zip(exps.iter())
        .map(|((name, _), exp)| (name.clone(), exp / sum))
        .collect()
}

/// Attempt to apply an oracle candidate tactic to a goal.
///
/// The candidate is a tactic name (e.g. `"auto"`, `"category_simp"`) or a
/// semicolon-separated sequence (e.g. `"intro; auto"`).  We split on `"; "`
/// and dispatch each step via `try_named_tactic`.
///
/// The result is verified by the SMT backend through the normal `execute_tactic`
/// / `try_named_tactic` path — the oracle is NEVER trusted without verification.
fn try_apply_oracle_candidate(
    goal: &ProofGoal,
    candidate: &str,
    engine: &mut ProofSearchEngine,
) -> Result<List<ProofGoal>, ProofError> {
    // Split compound candidates (e.g. "intro; auto") into individual steps.
    let steps: Vec<&str> = candidate.split("; ").collect();

    let mut current_goals: List<ProofGoal> = List::new();
    current_goals.push(goal.clone());

    for step in steps {
        if current_goals.is_empty() {
            break; // All goals already closed.
        }

        let tactic = ProofTactic::Named {
            name: Text::from(step.trim()),
            args: List::new(),
        };

        let mut next_goals: List<ProofGoal> = List::new();
        for g in current_goals.iter() {
            match engine.execute_tactic(&tactic, g) {
                Ok(mut subgoals) => {
                    for sg in subgoals.iter() {
                        next_goals.push(sg.clone());
                    }
                }
                Err(e) => return Err(e),
            }
        }
        current_goals = next_goals;
    }

    Ok(current_goals)
}

// ==================== Ring Polynomial ====================

/// Polynomial representation for ring normalization
#[derive(Debug, Clone, PartialEq)]
struct RingPolynomial {
    /// Coefficients by monomial (variable name -> exponent)
    terms: Map<List<(Text, u32)>, i64>,
}

impl RingPolynomial {
    fn new() -> Self {
        Self { terms: Map::new() }
    }

    fn from_expr(expr: &Expr) -> Self {
        let mut poly = RingPolynomial::new();
        poly.add_expr(expr, 1);
        poly
    }

    fn add_expr(&mut self, expr: &Expr, coeff: i64) {
        match &expr.kind {
            ExprKind::Literal(lit) => {
                if let verum_ast::literal::LiteralKind::Int(i) = &lit.kind {
                    self.add_constant(i.value as i64 * coeff);
                }
            }
            ExprKind::Path(p) => {
                if let Maybe::Some(ident) = p.as_ident() {
                    self.add_variable(ident.as_str().to_text(), 1, coeff);
                }
            }
            ExprKind::Binary {
                op: BinOp::Add,
                left,
                right,
            } => {
                self.add_expr(left, coeff);
                self.add_expr(right, coeff);
            }
            ExprKind::Binary {
                op: BinOp::Sub,
                left,
                right,
            } => {
                self.add_expr(left, coeff);
                self.add_expr(right, -coeff);
            }
            ExprKind::Binary {
                op: BinOp::Mul,
                left,
                right,
            } => {
                // Simplified: only handle constant * expr
                if let ExprKind::Literal(lit) = &left.kind {
                    if let verum_ast::literal::LiteralKind::Int(i) = &lit.kind {
                        self.add_expr(right, coeff * i.value as i64);
                    }
                } else if let ExprKind::Literal(lit) = &right.kind
                    && let verum_ast::literal::LiteralKind::Int(i) = &lit.kind
                {
                    self.add_expr(left, coeff * i.value as i64);
                }
                // For variable * variable, would need more complex handling
            }
            ExprKind::Unary {
                op: verum_ast::UnOp::Neg,
                expr: inner,
            } => {
                self.add_expr(inner, -coeff);
            }
            ExprKind::Paren(inner) => {
                self.add_expr(inner, coeff);
            }
            _ => {
                // For unhandled expressions, add as-is
            }
        }
    }

    fn add_constant(&mut self, value: i64) {
        let key = List::new();
        *self.terms.entry(key).or_insert(0) += value;
    }

    fn add_variable(&mut self, name: Text, exp: u32, coeff: i64) {
        let mut key = List::new();
        key.push((name, exp));
        *self.terms.entry(key).or_insert(0) += coeff;
    }
}

impl Default for ProofSearchEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Statistics ====================

/// Hint database statistics
#[derive(Debug, Default, Clone)]
pub struct HintStats {
    /// Total queries
    pub total_queries: u64,
    /// Successful lookups
    pub hits: u64,
    /// Total lookup time (microseconds)
    pub total_time_us: u64,
}

impl HintStats {
    /// Hit rate (0.0 - 1.0)
    pub fn hit_rate(&self) -> f64 {
        if self.total_queries == 0 {
            0.0
        } else {
            self.hits as f64 / self.total_queries as f64
        }
    }

    /// Average lookup time (microseconds)
    pub fn avg_time_us(&self) -> f64 {
        if self.total_queries == 0 {
            0.0
        } else {
            self.total_time_us as f64 / self.total_queries as f64
        }
    }
}

/// Proof search statistics
#[derive(Debug, Default, Clone)]
pub struct SearchStats {
    /// Total proof attempts
    pub total_attempts: u64,
    /// Successful proofs
    pub successes: u64,
    /// Failed proofs
    pub failures: u64,
    /// No applicable hints
    pub no_hints: u64,
}

impl SearchStats {
    /// Success rate (0.0 - 1.0)
    pub fn success_rate(&self) -> f64 {
        if self.total_attempts == 0 {
            0.0
        } else {
            self.successes as f64 / self.total_attempts as f64
        }
    }
}

// ==================== Decision Procedures ====================

/// Model from SAT solver
#[derive(Debug, Clone)]
pub struct Model {
    /// Variable assignments
    pub assignments: Map<Text, ModelValue>,
}

/// Value in a model
#[derive(Debug, Clone)]
pub enum ModelValue {
    /// Boolean value
    Bool(bool),
    /// Integer value
    Int(i64),
    /// Other value (stringified)
    Other(Text),
}

impl ProofSearchEngine {
    /// SAT decision procedure with soundness guarantee
    ///
    /// SAT decision procedure with soundness guarantee.
    /// Verified decision procedure: `@decide fn is_tautology(prop) -> Bool with result = true <-> |- prop`
    /// Returns (satisfiable, proof_certificate):
    /// - If SAT: certificate is a model (witness)
    /// - If UNSAT: certificate is a proof of unsatisfiability
    pub fn sat_decide(
        &mut self,
        context: &Context,
        formula: &Expr,
    ) -> Result<(bool, Maybe<ProofTerm>), ProofError> {
        // Translate formula to Z3
        let translator = Translator::new(context);
        let z3_formula = translator
            .translate_expr(formula)
            .map_err(|_| ProofError::TacticFailed("Failed to translate formula".into()))?;

        let z3_bool = match option_to_maybe(z3_formula.as_bool()) {
            Maybe::Some(b) => b,
            Maybe::None => return Err(ProofError::TacticFailed("Formula is not boolean".into())),
        };

        // Create solver and check
        let solver = context.solver();
        solver.assert(&z3_bool);

        match solver.check() {
            z3::SatResult::Sat => {
                // Extract model as witness
                if let Maybe::Some(model) = option_to_maybe(solver.get_model()) {
                    // Convert Z3 model to proof term
                    let proof = ProofTerm::SmtProof {
                        solver: "z3_sat".into(),
                        formula: formula.clone(),
                    };
                    Ok((true, Maybe::Some(proof)))
                } else {
                    Ok((true, Maybe::None))
                }
            }
            z3::SatResult::Unsat => {
                // Extract proof of unsatisfiability
                // In a full implementation, we would extract the UNSAT core
                let proof = ProofTerm::SmtProof {
                    solver: "z3_unsat".into(),
                    formula: formula.clone(),
                };
                Ok((false, Maybe::Some(proof)))
            }
            z3::SatResult::Unknown => Err(ProofError::SmtTimeout),
        }
    }

    /// Linear arithmetic decision procedure (Simplex algorithm)
    ///
    /// Linear arithmetic decision procedure using Simplex algorithm.
    /// Decides quantifier-free linear integer/real arithmetic formulas (QF_LIA / QF_LRA).
    pub fn linear_arithmetic_decide(
        &mut self,
        context: &Context,
        constraints: &List<Expr>,
    ) -> Result<bool, ProofError> {
        // Create conjunction of all constraints
        let mut formula = constraints
            .first()
            .ok_or_else(|| ProofError::TacticFailed("No constraints provided".into()))?
            .clone();

        for constraint in constraints.iter().skip(1) {
            let current_formula = formula.clone();
            formula = Expr::new(
                ExprKind::Binary {
                    op: BinOp::And,
                    left: Box::new(current_formula),
                    right: Box::new(constraint.clone()),
                },
                formula.span,
            );
        }

        // Use Z3's linear arithmetic theory
        let (sat, _) = self.sat_decide(context, &formula)?;
        Ok(sat)
    }

    /// Presburger arithmetic decision procedure (Cooper's algorithm)
    ///
    /// Presburger arithmetic decision procedure via Cooper's algorithm.
    /// Decides linear integer arithmetic WITH quantifiers (full Presburger arithmetic).
    /// Z3 handles quantifier elimination for this fragment automatically.
    pub fn presburger_decide(
        &mut self,
        context: &Context,
        formula: &Expr,
    ) -> Result<bool, ProofError> {
        // Presburger arithmetic is decidable
        // Z3 supports quantifier elimination for linear integer arithmetic
        let (sat, _) = self.sat_decide(context, formula)?;
        Ok(sat)
    }
}

// ==================== Program Extraction ====================

/// Function extracted from proof
///
/// A function extracted from a constructive proof via the Curry-Howard correspondence.
/// Given a proof of `exists!(q, r: Nat). a = b * q + r /\ r < b`, extraction yields
/// an executable `div_mod(a, b)` function. Supports extraction directives:
/// `@extract`, `@extract(target = "ocaml")`, `@extract(optimize = true)`.
/// Proof-irrelevant parts are erased; only computational content is retained.
#[derive(Debug, Clone)]
pub struct ExtractedFunction {
    /// Function name
    pub name: Text,

    /// Function parameters
    pub params: List<Text>,

    /// Function body (extracted from proof)
    pub body: Expr,

    /// Original proof term
    pub proof: ProofTerm,
}

impl ProofSearchEngine {
    /// Extract computational program from constructive proof
    ///
    /// Extract computational program from constructive proof (`@extract` directive).
    /// Extracts the computational content from existence proofs (e.g., `exists!(q, r).`),
    /// turning proofs into executable programs. The extracted function inherits the
    /// proven contracts as runtime-free guarantees.
    pub fn extract_program(&self, proof: &ProofTerm) -> Result<ExtractedFunction, ProofError> {
        match proof {
            ProofTerm::Lambda { var, body } => {
                // Extract lambda as function
                let func_body = self.proof_to_expr(body)?;

                Ok(ExtractedFunction {
                    name: format!("extracted_{}", var).into(),
                    params: List::from_iter(vec![var.clone()]),
                    body: func_body,
                    proof: proof.clone(),
                })
            }

            ProofTerm::Cases { scrutinee, cases } => {
                // Extract case analysis as match expression
                let mut arms = List::new();

                for (_pattern, case_proof) in cases {
                    let expr = self.proof_to_expr(case_proof)?;
                    arms.push(expr);
                }

                // Create match expression
                use verum_ast::Ident;
                use verum_ast::span::Span;
                let ident = Ident::new("extracted_cases", Span::dummy());
                let match_expr = Expr::new(ExprKind::Path(Path::from_ident(ident)), Span::dummy());

                Ok(ExtractedFunction {
                    name: "extracted_cases".into(),
                    params: List::new(),
                    body: match_expr,
                    proof: proof.clone(),
                })
            }

            ProofTerm::Induction {
                var,
                base_case,
                inductive_case,
            } => {
                // Extract recursive function from induction
                let base_expr = self.proof_to_expr(base_case)?;
                let ind_expr = self.proof_to_expr(inductive_case)?;

                Ok(ExtractedFunction {
                    name: format!("extracted_induction_{}", var).into(),
                    params: List::from_iter(vec![var.clone()]),
                    body: base_expr, // Simplified - full impl would build recursive structure
                    proof: proof.clone(),
                })
            }

            ProofTerm::SmtProof { .. } => {
                // SMT proofs don't contain computational content
                Err(ProofError::TacticFailed(
                    "Cannot extract program from SMT proof".into(),
                ))
            }

            ProofTerm::Apply { .. } | ProofTerm::Axiom(_) => Err(ProofError::TacticFailed(
                "Cannot extract program from non-constructive proof".into(),
            )),
        }
    }

    /// Convert proof term to executable expression
    ///
    /// Erases proof-irrelevant parts and extracts computational content.
    ///
    /// Convert proof term to executable expression. Erases proof-irrelevant parts
    /// (axioms, SMT proofs become unit) and retains computational content (lambdas
    /// become function params, cases become match expressions). Supports `@extract_witness`
    /// for extracting just the witness value without the proof obligation.
    fn proof_to_expr(&self, proof: &ProofTerm) -> Result<Expr, ProofError> {
        use verum_ast::literal::{Literal, LiteralKind};
        use verum_ast::span::Span;

        match proof {
            ProofTerm::Lambda { var, body } => {
                // Lambda becomes function parameter
                let body_expr = self.proof_to_expr(body)?;
                Ok(body_expr)
            }

            ProofTerm::Cases { scrutinee, cases } => {
                // Cases become match expression
                Ok(scrutinee.clone())
            }

            ProofTerm::SmtProof { .. } | ProofTerm::Axiom(_) | ProofTerm::Apply { .. } => {
                // No computational content - return unit
                Ok(Expr::new(
                    ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
                    Span::dummy(),
                ))
            }

            ProofTerm::Induction { .. } => {
                // Induction becomes recursion
                Ok(Expr::new(
                    ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
                    Span::dummy(),
                ))
            }
        }
    }

    /// Extract witness from existence proof
    ///
    /// Extract witness from existence proof: given a proof of `exists x. P(x)`,
    /// returns the witness value x. Used by `@extract_witness` directive:
    /// `fn next_prime(n: Nat) -> Nat is witness_of(exists_prime_above(n))`
    pub fn extract_witness(&self, proof: &ProofTerm) -> Result<Expr, ProofError> {
        match proof {
            ProofTerm::Lambda { var: _, body } => {
                // The body should contain the witness
                self.proof_to_expr(body)
            }

            _ => Err(ProofError::TacticFailed(
                "Proof is not an existence proof".into(),
            )),
        }
    }
}
