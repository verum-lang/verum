//! Program Extraction from Proofs
//!
//! This module implements extraction of computational content from constructive proofs
//! implementing the Curry-Howard correspondence for extracting computational content from proofs.
//!
//! ## Features
//!
//! - **Program Extraction**: Extract executable code from constructive proofs
//! - **Witness Extraction**: Extract witnesses from existential proofs
//! - **Proof Irrelevance**: Mark and erase proof-irrelevant parts during extraction
//! - **Contract Generation**: Generate runtime contracts from proof obligations
//! - **Multi-Target Support**: Extract to Verum, OCaml, or other targets
//!
//! ## Extraction Process
//!
//! 1. **Analyze Proof**: Determine if proof is constructive and extractable
//! 2. **Extract Computational Content**: Convert proof terms to program terms
//! 3. **Erase Proofs**: Remove proof-irrelevant parts for runtime efficiency
//! 4. **Generate Contracts**: Convert proof obligations to runtime checks
//! 5. **Emit Target Code**: Generate code in target language
//!
//! ## Example Usage
//!
//! ```rust,ignore
//! use verum_smt::program_extraction::{ProgramExtractor, ExtractionTarget};
//! use verum_smt::proof_term_unified::ProofTerm;
//!
//! let extractor = ProgramExtractor::new();
//!
//! // Extract function from proof of existence and uniqueness
//! // theorem div_mod_unique(a, b: Nat, b > 0):
//! //     ∃!(q, r: Nat). a = b * q + r ∧ r < b
//! let proof = /* ... constructive proof ... */;
//!
//! if let Some(program) = extractor.extract_program(&proof) {
//!     println!("Extracted program: {:?}", program);
//! }
//! ```
//!
//! Program extraction: `@extract` on constructive proofs generates executable code.
//! `@extract_witness` extracts witnesses without proofs. `@extract_contract` generates
//! runtime contracts. Proof-irrelevant (Prop-typed) components are erased.

use verum_ast::expr::RecoverBody;
use verum_ast::span::Span;
use verum_ast::{
    BinOp, ContextList, Expr, ExprKind, Literal, LiteralKind, Pattern, PatternKind, Type, TypeKind,
    UnOp,
};
use verum_common::{Heap, List, Maybe, Text};
use verum_common::ToText;

use crate::proof_term_unified::ProofTerm;

// ==================== Extraction Configuration ====================

/// Target language for code extraction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionTarget {
    /// Verum native code
    Verum,
    /// OCaml code (for Coq integration)
    OCaml,
    /// Lean code
    Lean,
    /// Coq code
    Coq,
}

impl ExtractionTarget {
    /// Get file extension for target
    pub fn file_extension(&self) -> &'static str {
        match self {
            Self::Verum => "vr",
            Self::OCaml => "ml",
            Self::Lean => "lean",
            Self::Coq => "v",
        }
    }

    /// Get language name
    pub fn language_name(&self) -> &'static str {
        match self {
            Self::Verum => "Verum",
            Self::OCaml => "OCaml",
            Self::Lean => "Lean",
            Self::Coq => "Coq",
        }
    }
}

/// Configuration for program extraction
#[derive(Debug, Clone)]
pub struct ExtractionConfig {
    /// Target language for extraction
    pub target: ExtractionTarget,

    /// Whether to optimize extracted code
    pub optimize: bool,

    /// Whether to erase proof-irrelevant terms
    pub erase_proofs: bool,

    /// Whether to generate runtime contracts from proofs
    pub generate_contracts: bool,

    /// Whether to inline small functions
    pub inline_small_functions: bool,

    /// Maximum function size for inlining (in AST nodes)
    pub inline_threshold: usize,

    /// Whether to generate documentation from proofs
    pub generate_docs: bool,
}

impl ExtractionConfig {
    /// Create default extraction configuration
    pub fn new() -> Self {
        Self {
            target: ExtractionTarget::Verum,
            optimize: true,
            erase_proofs: true,
            generate_contracts: true,
            inline_small_functions: true,
            inline_threshold: 20,
            generate_docs: true,
        }
    }

    /// Create configuration for target language
    pub fn for_target(target: ExtractionTarget) -> Self {
        let mut config = Self::new();
        config.target = target;
        config
    }

    /// Disable optimizations (useful for debugging)
    pub fn without_optimizations(mut self) -> Self {
        self.optimize = false;
        self.inline_small_functions = false;
        self
    }

    /// Keep proofs (don't erase, useful for proof certificates)
    pub fn keep_proofs(mut self) -> Self {
        self.erase_proofs = false;
        self
    }
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Extracted Program Representation ====================

/// Extracted program from a proof
///
/// Represents executable code extracted from a constructive proof.
/// The program may contain contracts derived from proof obligations.
#[derive(Debug, Clone)]
pub struct ExtractedProgram {
    /// Function name
    pub name: Text,

    /// Function parameters
    pub params: List<Parameter>,

    /// Return type
    pub return_type: Type,

    /// Function body
    pub body: Expr,

    /// Preconditions (from proof assumptions)
    pub preconditions: List<Contract>,

    /// Postconditions (from proof conclusion)
    pub postconditions: List<Contract>,

    /// Whether this program was extracted from a proof
    pub is_extracted: bool,

    /// Source proof term (if available)
    pub source_proof: Maybe<Heap<ProofTerm>>,

    /// Documentation extracted from proof
    pub documentation: Maybe<Text>,
}

impl ExtractedProgram {
    /// Create a new extracted program
    pub fn new(name: Text, params: List<Parameter>, return_type: Type, body: Expr) -> Self {
        Self {
            name,
            params,
            return_type,
            body,
            preconditions: List::new(),
            postconditions: List::new(),
            is_extracted: true,
            source_proof: Maybe::None,
            documentation: Maybe::None,
        }
    }

    /// Add precondition
    pub fn with_precondition(mut self, precondition: Contract) -> Self {
        self.preconditions.push(precondition);
        self
    }

    /// Add postcondition
    pub fn with_postcondition(mut self, postcondition: Contract) -> Self {
        self.postconditions.push(postcondition);
        self
    }

    /// Set source proof
    pub fn with_source_proof(mut self, proof: ProofTerm) -> Self {
        self.source_proof = Maybe::Some(Heap::new(proof));
        self
    }

    /// Set documentation
    pub fn with_documentation(mut self, doc: Text) -> Self {
        self.documentation = Maybe::Some(doc);
        self
    }

    /// Count AST nodes in body (for optimization decisions)
    pub fn body_size(&self) -> usize {
        self.count_expr_nodes(&self.body)
    }

    fn count_expr_nodes(&self, expr: &Expr) -> usize {
        let mut count = 1;

        match &expr.kind {
            ExprKind::Binary { left, right, .. } => {
                count += self.count_expr_nodes(left);
                count += self.count_expr_nodes(right);
            }
            ExprKind::Unary { expr: inner, .. } => {
                count += self.count_expr_nodes(inner);
            }
            ExprKind::Call { func, args, .. } => {
                count += self.count_expr_nodes(func);
                for arg in args {
                    count += self.count_expr_nodes(arg);
                }
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Count condition expressions
                for cond_kind in &condition.conditions {
                    match cond_kind {
                        verum_ast::expr::ConditionKind::Expr(e) => {
                            count += self.count_expr_nodes(e)
                        }
                        verum_ast::expr::ConditionKind::Let { value, .. } => {
                            count += self.count_expr_nodes(value)
                        }
                    }
                }
                count += self.count_block_nodes(then_branch);
                if let Maybe::Some(else_expr_heap) = else_branch {
                    count += self.count_expr_nodes(else_expr_heap);
                }
            }
            ExprKind::Block(block) => {
                count += self.count_block_nodes(block);
            }
            _ => {}
        }

        count
    }

    fn count_block_nodes(&self, block: &verum_ast::Block) -> usize {
        let mut count = 0;
        for stmt in &block.stmts {
            count += 1; // Statement node
            // Could recursively count expression nodes in statements
        }
        if let Maybe::Some(expr) = &block.expr {
            count += self.count_expr_nodes(expr);
        }
        count
    }
}

/// Function parameter
#[derive(Debug, Clone)]
pub struct Parameter {
    /// Parameter name
    pub name: Text,

    /// Parameter type
    pub ty: Type,

    /// Whether parameter is implicit (from proof)
    pub is_implicit: bool,
}

impl Parameter {
    /// Create a new parameter
    pub fn new(name: Text, ty: Type) -> Self {
        Self {
            name,
            ty,
            is_implicit: false,
        }
    }

    /// Mark parameter as implicit
    pub fn implicit(mut self) -> Self {
        self.is_implicit = true;
        self
    }
}

/// Contract (pre/postcondition)
#[derive(Debug, Clone)]
pub struct Contract {
    /// Contract expression
    pub expr: Expr,

    /// Contract kind
    pub kind: ContractKind,

    /// Whether this is a runtime or compile-time contract
    pub is_runtime: bool,
}

impl Contract {
    /// Create a precondition contract
    pub fn precondition(expr: Expr) -> Self {
        Self {
            expr,
            kind: ContractKind::Precondition,
            is_runtime: true,
        }
    }

    /// Create a postcondition contract
    pub fn postcondition(expr: Expr) -> Self {
        Self {
            expr,
            kind: ContractKind::Postcondition,
            is_runtime: true,
        }
    }

    /// Create an invariant contract
    pub fn invariant(expr: Expr) -> Self {
        Self {
            expr,
            kind: ContractKind::Invariant,
            is_runtime: true,
        }
    }

    /// Mark as compile-time only
    pub fn compile_time(mut self) -> Self {
        self.is_runtime = false;
        self
    }
}

/// Contract kind
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractKind {
    /// Precondition (requires)
    Precondition,
    /// Postcondition (ensures)
    Postcondition,
    /// Loop invariant
    Invariant,
    /// Termination measure
    Measure,
}

// ==================== Witness Extraction ====================

/// Witness extracted from an existential proof
///
/// Represents a concrete value that satisfies an existential property.
/// Used for @extract_witness directive.
#[derive(Debug, Clone)]
pub struct ExtractedWitness {
    /// Witness name
    pub name: Text,

    /// Witness type
    pub ty: Type,

    /// Witness value (may be a function computing the witness)
    pub value: Expr,

    /// Property that witness satisfies
    pub property: Maybe<Expr>,

    /// Source proof
    pub source_proof: Maybe<Heap<ProofTerm>>,
}

impl ExtractedWitness {
    /// Create a new extracted witness
    pub fn new(name: Text, ty: Type, value: Expr) -> Self {
        Self {
            name,
            ty,
            value,
            property: Maybe::None,
            source_proof: Maybe::None,
        }
    }

    /// Set the property this witness satisfies
    pub fn with_property(mut self, property: Expr) -> Self {
        self.property = Maybe::Some(property);
        self
    }

    /// Set source proof
    pub fn with_source_proof(mut self, proof: ProofTerm) -> Self {
        self.source_proof = Maybe::Some(Heap::new(proof));
        self
    }
}

// ==================== Program Extractor ====================

/// Program extractor from proofs
///
/// Extracts executable programs from constructive proofs.
/// Implements the Curry-Howard correspondence between proofs and programs.
pub struct ProgramExtractor {
    /// Extraction configuration
    #[allow(dead_code)] // Reserved for extraction configuration
    config: ExtractionConfig,

    /// Statistics
    stats: ExtractionStats,
}

impl ProgramExtractor {
    /// Create a new program extractor
    pub fn new() -> Self {
        Self {
            config: ExtractionConfig::new(),
            stats: ExtractionStats::default(),
        }
    }

    /// Create extractor with custom configuration
    pub fn with_config(config: ExtractionConfig) -> Self {
        Self {
            config,
            stats: ExtractionStats::default(),
        }
    }

    /// Extract program from a proof term
    ///
    /// Returns Some(program) if the proof is constructive and can be extracted,
    /// None otherwise.
    ///
    /// Extract executable function from constructive existence proof via `@extract`.
    pub fn extract_program(&mut self, proof: &ProofTerm) -> Maybe<ExtractedProgram> {
        self.stats.attempts += 1;

        // Check if proof is extractable (constructive)
        if !self.is_extractable(proof) {
            self.stats.non_extractable += 1;
            return Maybe::None;
        }

        // Extract computational content
        let result = self.extract_computational_content(proof);

        if result.is_some() {
            self.stats.successful += 1;
        }

        result
    }

    /// Extract witness from an existential proof
    ///
    /// Extracts the witness term from a proof of ∃x. P(x).
    /// Used for @extract_witness directive.
    ///
    /// Extract witness term from existential proof via `@extract_witness`.
    pub fn extract_witness(&mut self, proof: &ProofTerm) -> Maybe<ExtractedWitness> {
        self.stats.witness_extractions += 1;

        match proof {
            // Lambda body may contain witness
            ProofTerm::Lambda { body, .. } => {
                // Extract witness from body
                if let Maybe::Some(witness_expr) = self.proof_term_to_expr(body)
                    && let Maybe::Some(witness_type) = self.infer_type(&witness_expr)
                {
                    return Maybe::Some(
                        ExtractedWitness::new("witness".to_text(), witness_type, witness_expr)
                            .with_source_proof((**body).clone()),
                    );
                }
                Maybe::None
            }

            // Cases may contain existential in branches
            ProofTerm::Cases { scrutinee, cases } => {
                // Try to extract from first case that contains witness
                for (_pattern, proof_term) in cases {
                    if let Maybe::Some(witness) = self.extract_witness(proof_term) {
                        return Maybe::Some(witness);
                    }
                }
                Maybe::None
            }

            // Lemma may wrap existential proof
            ProofTerm::Lemma {
                conclusion: _,
                proof,
            } => self.extract_witness(proof),

            _ => {
                // Cannot extract witness from this proof structure
                Maybe::None
            }
        }
    }

    /// Check if a proof is extractable (constructive)
    fn is_extractable(&self, proof: &ProofTerm) -> bool {
        match proof {
            // Constructive proof terms
            ProofTerm::Lambda { .. }
            | ProofTerm::Apply { .. }
            | ProofTerm::Cases { .. }
            | ProofTerm::Induction { .. } => true,

            // Axioms may or may not be extractable
            ProofTerm::Axiom { .. } => false,

            // Classical reasoning is generally not extractable
            ProofTerm::TheoryLemma { .. }
            | ProofTerm::UnitResolution { .. }
            | ProofTerm::SmtProof { .. } => false,

            // Recursively check sub-proofs
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => self.is_extractable(premise) && self.is_extractable(implication),

            ProofTerm::Lemma { proof, .. } => self.is_extractable(proof),

            ProofTerm::Transitivity { left, right } => {
                self.is_extractable(left) && self.is_extractable(right)
            }

            // Other cases are not extractable
            _ => false,
        }
    }

    /// Extract computational content from a proof
    fn extract_computational_content(&mut self, proof: &ProofTerm) -> Maybe<ExtractedProgram> {
        match proof {
            // Lambda abstraction becomes function
            ProofTerm::Lambda { var, body } => {
                let param_type = self.infer_param_type(var)?;
                let body_expr = self.proof_term_to_expr(body)?;
                let return_type = self.infer_type(&body_expr)?;

                let mut params = List::new();
                params.push(Parameter::new(var.clone(), param_type));

                Maybe::Some(ExtractedProgram::new(
                    "extracted".to_text(),
                    params,
                    return_type,
                    body_expr,
                ))
            }

            // Cases become pattern matching
            ProofTerm::Cases { scrutinee, cases } => {
                let scrutinee_expr = scrutinee.clone();
                let mut match_arms = List::new();

                for (pattern_expr, proof_term) in cases {
                    if let Maybe::Some(pattern) = self.expr_to_pattern(pattern_expr)
                        && let Maybe::Some(body_expr) = self.proof_term_to_expr(proof_term)
                    {
                        match_arms.push(verum_ast::MatchArm {
                            pattern,
                            guard: Maybe::None,
                            body: Heap::new(body_expr),
                            with_clause: Maybe::None,
                            attributes: List::new(),
                            span: Span::default(),
                        });
                    }
                }

                // Create match expression
                let match_expr = Expr::new(
                    ExprKind::Match {
                        expr: Heap::new(scrutinee_expr),
                        arms: match_arms,
                    },
                    Span::default(),
                );

                let return_type = self.infer_type(&match_expr)?;

                Maybe::Some(ExtractedProgram::new(
                    "extracted".to_text(),
                    List::new(),
                    return_type,
                    match_expr,
                ))
            }

            // Induction becomes recursive function
            ProofTerm::Induction {
                var,
                base_case,
                inductive_case,
            } => {
                // Extract base case
                let base_expr = self.proof_term_to_expr(base_case)?;

                // Extract inductive case
                let inductive_expr = self.proof_term_to_expr(inductive_case)?;

                // Build recursive function structure using Curry-Howard correspondence
                // Transforms induction proof into a recursive function:
                // - Base case becomes base return value
                // - Inductive step becomes recursive step with IH as recursive call
                let body = self.build_recursive_function(var, &base_expr, &inductive_expr)?;
                let return_type = self.infer_type(&body)?;

                let mut params = List::new();
                params.push(Parameter::new(var.clone(), self.nat_type()));

                Maybe::Some(ExtractedProgram::new(
                    "extracted_recursive".to_text(),
                    params,
                    return_type,
                    body,
                ))
            }

            // Application becomes function application
            ProofTerm::Apply { rule, premises } => {
                // Simplified: use rule name and first premise
                if let Some(first_premise) = premises.first() {
                    let premise_expr = self.proof_term_to_expr(first_premise)?;
                    let return_type = self.infer_type(&premise_expr)?;

                    Maybe::Some(ExtractedProgram::new(
                        rule.clone(),
                        List::new(),
                        return_type,
                        premise_expr,
                    ))
                } else {
                    Maybe::None
                }
            }

            // Induction or cases can be witness sources
            // (Note: Sigma types would be handled here if we had them in ProofTerm)

            // Lemma: extract from wrapped proof
            ProofTerm::Lemma {
                conclusion: _,
                proof,
            } => self.extract_computational_content(proof),

            _ => {
                // Not extractable
                Maybe::None
            }
        }
    }

    /// Convert proof term to expression
    fn proof_term_to_expr(&self, proof: &ProofTerm) -> Maybe<Expr> {
        match proof {
            ProofTerm::Axiom { name, formula } => {
                // Use formula as-is if it's already an expression
                Maybe::Some(formula.clone())
            }

            ProofTerm::Lambda { var, body } => {
                let body_expr = self.proof_term_to_expr(body)?;

                // Create closure expression
                let mut params = List::new();
                params.push(verum_ast::expr::ClosureParam::new(
                    Pattern::new(
                        PatternKind::Ident {
                            by_ref: false,
                            mutable: false,
                            name: verum_ast::ty::Ident {
                                name: var.as_str().to_string().into(),
                                span: Span::default(),
                            },
                            subpattern: Maybe::None,
                        },
                        Span::default(),
                    ),
                    Maybe::None,
                    Span::default(),
                ));

                let closure = Expr::new(
                    ExprKind::Closure {
                        async_: false,
                        move_: false,
                        params,
                        contexts: List::new(),
                        return_type: Maybe::None,
                        body: Heap::new(body_expr),
                    },
                    Span::default(),
                );

                Maybe::Some(closure)
            }

            ProofTerm::Apply { rule: _, premises } => {
                // Use first premise as the result
                if let Some(first) = premises.first() {
                    self.proof_term_to_expr(first)
                } else {
                    Maybe::None
                }
            }

            ProofTerm::Cases { scrutinee, cases } => {
                let scrutinee_expr = scrutinee.clone();
                let mut match_arms = List::new();

                for (pattern_expr, proof_term) in cases {
                    if let Maybe::Some(pattern) = self.expr_to_pattern(pattern_expr)
                        && let Maybe::Some(body_expr) = self.proof_term_to_expr(proof_term)
                    {
                        match_arms.push(verum_ast::MatchArm {
                            pattern,
                            guard: Maybe::None,
                            body: Heap::new(body_expr),
                            with_clause: Maybe::None,
                            attributes: List::new(),
                            span: Span::default(),
                        });
                    }
                }

                Maybe::Some(Expr::new(
                    ExprKind::Match {
                        expr: Heap::new(scrutinee_expr),
                        arms: match_arms,
                    },
                    Span::default(),
                ))
            }

            _ => {
                // Create placeholder or error
                Maybe::None
            }
        }
    }

    /// Extract match arms from proof cases
    #[allow(dead_code)] // Part of proof extraction API
    fn extract_match_arms_from_cases(
        &self,
        cases: &List<(Expr, Heap<ProofTerm>)>,
    ) -> Maybe<List<verum_ast::MatchArm>> {
        let mut match_arms = List::new();

        for (pattern_expr, proof_term) in cases {
            // Try to convert expression to pattern
            let pattern = self.expr_to_pattern(pattern_expr)?;
            let body_expr = self.proof_term_to_expr(proof_term)?;

            match_arms.push(verum_ast::MatchArm {
                pattern,
                guard: Maybe::None,
                body: Heap::new(body_expr),
                with_clause: Maybe::None,
                attributes: List::new(),
                span: Span::default(),
            });
        }

        Maybe::Some(match_arms)
    }

    /// Convert expression to pattern
    ///
    /// Handles all pattern types according to Verum's pattern grammar:
    /// - Wildcards: `_`
    /// - Identifiers: `x`, `mut x`
    /// - Literals: `42`, `"hello"`, `true`
    /// - Tuples: `(a, b, c)`
    /// - Arrays: `[a, b, c]`
    /// - Records: `Point { x, y }`
    /// - Variants: `Some(x)`, `None`
    /// - Or patterns: `a | b`
    /// - References: `&x`, `&mut x`
    /// - Ranges: `1..10`, `1..=10`
    ///
    /// Pattern extraction: literal, wildcard, binding, variant, or, reference, range patterns.
    fn expr_to_pattern(&self, expr: &Expr) -> Maybe<Pattern> {
        match &expr.kind {
            // Path expressions become identifier or variant patterns
            ExprKind::Path(path) => {
                if let Some(last_seg) = path.segments.last() {
                    match last_seg {
                        verum_ast::ty::PathSegment::Name(ident) => {
                            // Check for wildcard
                            if ident.name.as_str() == "_" {
                                return Maybe::Some(Pattern::wildcard(expr.span));
                            }
                            // Check for boolean literals that might be parsed as paths
                            if ident.name.as_str() == "true" || ident.name.as_str() == "false" {
                                let bool_val = ident.name.as_str() == "true";
                                return Maybe::Some(Pattern::literal(Literal {
                                    kind: LiteralKind::Bool(bool_val),
                                    span: expr.span,
                                }));
                            }
                            // For multi-segment paths, treat as variant pattern without data
                            if path.segments.len() > 1 {
                                return Maybe::Some(Pattern::new(
                                    PatternKind::Variant {
                                        path: path.clone(),
                                        data: Maybe::None,
                                    },
                                    expr.span,
                                ));
                            }
                            // Single identifier pattern
                            Maybe::Some(Pattern::new(
                                PatternKind::Ident {
                                    by_ref: false,
                                    mutable: false,
                                    name: ident.clone(),
                                    subpattern: Maybe::None,
                                },
                                expr.span,
                            ))
                        }
                        // Handle self, super, crate as identifiers
                        verum_ast::ty::PathSegment::SelfValue => Maybe::Some(Pattern::new(
                            PatternKind::Ident {
                                by_ref: false,
                                mutable: false,
                                name: verum_ast::ty::Ident {
                                    name: "self".to_string().into(),
                                    span: expr.span,
                                },
                                subpattern: Maybe::None,
                            },
                            expr.span,
                        )),
                        _ => Maybe::None,
                    }
                } else {
                    Maybe::None
                }
            }

            // Literal patterns
            ExprKind::Literal(lit) => Maybe::Some(Pattern::literal(lit.clone())),

            // Tuple patterns: (a, b, c)
            ExprKind::Tuple(elements) => {
                let mut patterns = List::new();
                for elem in elements {
                    if let Maybe::Some(pat) = self.expr_to_pattern(elem) {
                        patterns.push(pat);
                    } else {
                        return Maybe::None;
                    }
                }
                Maybe::Some(Pattern::new(PatternKind::Tuple(patterns), expr.span))
            }

            // Array patterns: [a, b, c]
            ExprKind::Array(array_expr) => match array_expr {
                verum_ast::ArrayExpr::List(elements) => {
                    let mut patterns = List::new();
                    for elem in elements {
                        if let Maybe::Some(pat) = self.expr_to_pattern(elem) {
                            patterns.push(pat);
                        } else {
                            return Maybe::None;
                        }
                    }
                    Maybe::Some(Pattern::new(PatternKind::Array(patterns), expr.span))
                }
                _ => Maybe::None, // Repeat syntax not valid in patterns
            },

            // Record patterns: Point { x, y }
            ExprKind::Record { path, fields, base } => {
                // Base/spread not allowed in patterns
                if base.is_some() {
                    return Maybe::None;
                }
                let mut field_patterns = List::new();
                for field in fields {
                    let field_pattern = if let Maybe::Some(ref value_expr) = field.value {
                        // Point { x: pattern }
                        if let Maybe::Some(pat) = self.expr_to_pattern(value_expr) {
                            verum_ast::pattern::FieldPattern::new(
                                field.name.clone(),
                                Maybe::Some(pat),
                                field.span,
                            )
                        } else {
                            return Maybe::None;
                        }
                    } else {
                        // Point { x } shorthand
                        verum_ast::pattern::FieldPattern::shorthand(field.name.clone())
                    };
                    field_patterns.push(field_pattern);
                }
                Maybe::Some(Pattern::new(
                    PatternKind::Record {
                        path: path.clone(),
                        fields: field_patterns,
                        rest: false,
                    },
                    expr.span,
                ))
            }

            // Function call expressions can become variant patterns: Some(x)
            ExprKind::Call { func, args, .. } => {
                if let ExprKind::Path(path) = &func.kind {
                    // Convert arguments to patterns
                    let mut arg_patterns = List::new();
                    for arg in args {
                        if let Maybe::Some(pat) = self.expr_to_pattern(arg) {
                            arg_patterns.push(pat);
                        } else {
                            return Maybe::None;
                        }
                    }
                    Maybe::Some(Pattern::new(
                        PatternKind::Variant {
                            path: path.clone(),
                            data: Maybe::Some(verum_ast::pattern::VariantPatternData::Tuple(
                                arg_patterns,
                            )),
                        },
                        expr.span,
                    ))
                } else {
                    Maybe::None
                }
            }

            // Binary or expression becomes or pattern: a | b
            ExprKind::Binary { op, left, right } => {
                if *op == BinOp::BitOr {
                    // Convert to or pattern
                    let left_pat = self.expr_to_pattern(left)?;
                    let right_pat = self.expr_to_pattern(right)?;
                    // Flatten nested or patterns
                    let mut alternatives = List::new();
                    if let PatternKind::Or(left_alts) = left_pat.kind {
                        for alt in left_alts {
                            alternatives.push(alt);
                        }
                    } else {
                        alternatives.push(left_pat);
                    }
                    if let PatternKind::Or(right_alts) = right_pat.kind {
                        for alt in right_alts {
                            alternatives.push(alt);
                        }
                    } else {
                        alternatives.push(right_pat);
                    }
                    Maybe::Some(Pattern::new(PatternKind::Or(alternatives), expr.span))
                } else {
                    Maybe::None
                }
            }

            // Unary reference expressions become reference patterns: &x, &mut x
            ExprKind::Unary { op, expr: inner } => match op {
                UnOp::Ref => {
                    let inner_pat = self.expr_to_pattern(inner)?;
                    Maybe::Some(Pattern::new(
                        PatternKind::Reference {
                            mutable: false,
                            inner: Heap::new(inner_pat),
                        },
                        expr.span,
                    ))
                }
                UnOp::RefMut => {
                    let inner_pat = self.expr_to_pattern(inner)?;
                    Maybe::Some(Pattern::new(
                        PatternKind::Reference {
                            mutable: true,
                            inner: Heap::new(inner_pat),
                        },
                        expr.span,
                    ))
                }
                _ => Maybe::None,
            },

            // Range expressions become range patterns: 1..10, 1..=10
            ExprKind::Range {
                start,
                end,
                inclusive,
            } => {
                let start_lit = if let Maybe::Some(s) = start {
                    if let ExprKind::Literal(lit) = &s.kind {
                        Maybe::Some(Heap::new(lit.clone()))
                    } else {
                        Maybe::None
                    }
                } else {
                    Maybe::None
                };
                let end_lit = if let Maybe::Some(e) = end {
                    if let ExprKind::Literal(lit) = &e.kind {
                        Maybe::Some(Heap::new(lit.clone()))
                    } else {
                        Maybe::None
                    }
                } else {
                    Maybe::None
                };
                Maybe::Some(Pattern::new(
                    PatternKind::Range {
                        start: start_lit,
                        end: end_lit,
                        inclusive: *inclusive,
                    },
                    expr.span,
                ))
            }

            // Parenthesized expressions preserve the inner pattern
            ExprKind::Paren(inner) => {
                let inner_pat = self.expr_to_pattern(inner)?;
                Maybe::Some(Pattern::new(
                    PatternKind::Paren(Heap::new(inner_pat)),
                    expr.span,
                ))
            }

            // Other expression kinds cannot be converted to patterns
            _ => Maybe::None,
        }
    }

    /// Create pattern from text
    #[allow(dead_code)] // Part of proof extraction API
    fn create_pattern(&self, name: &Text) -> Maybe<Pattern> {
        // Simple identifier pattern
        Maybe::Some(Pattern::new(
            PatternKind::Ident {
                by_ref: false,
                mutable: false,
                name: verum_ast::ty::Ident {
                    name: name.as_str().to_string().into(),
                    span: Span::default(),
                },
                subpattern: Maybe::None,
            },
            Span::default(),
        ))
    }

    /// Create function parameter
    #[allow(dead_code)] // Part of proof extraction API
    fn create_param(&self, name: Text) -> verum_ast::decl::FunctionParam {
        verum_ast::decl::FunctionParam {
            kind: verum_ast::decl::FunctionParamKind::Regular {
                pattern: Pattern::ident(
                    verum_ast::ty::Ident {
                        name: name.as_str().to_string().into(),
                        span: Span::default(),
                    },
                    false, // not mutable
                    Span::default(),
                ),
                ty: self.unknown_type(),
                default_value: Maybe::None,
            },
            attributes: List::new(),
            span: Span::default(),
        }
    }

    /// Infer parameter type from proof context
    ///
    /// This function infers types for parameters based on:
    /// 1. Naming conventions (common mathematical patterns)
    /// 2. Context from proof structure
    /// 3. Default to inferred type when uncertain
    ///
    /// Base types: Bool, Int (arbitrary precision), Float (IEEE 754), Text (UTF-8), Unit.
    fn infer_param_type(&self, param: &Text) -> Maybe<Type> {
        let name = param.as_str();

        // Check for common mathematical/type naming conventions
        match name {
            // Natural numbers
            "n" | "m" | "k" | "i" | "j" => Maybe::Some(self.nat_type()),

            // Boolean parameters
            "b" | "p" | "q" | "cond" | "flag" => Maybe::Some(Type::bool(Span::default())),

            // Integer parameters
            "x" | "y" | "z" | "a" | "c" | "d" => Maybe::Some(Type::int(Span::default())),

            // Floating point parameters
            "f" | "g" | "t" | "s" | "u" | "v" | "w" => Maybe::Some(Type::float(Span::default())),

            // String/text parameters
            "str" | "text" | "msg" | "name" | "path" => Maybe::Some(Type::text(Span::default())),

            // Array/list elements (often used in recursion proofs)
            "xs" | "ys" | "zs" | "list" | "arr" => {
                // List of inferred element type
                let elem_type = Type::inferred(Span::default());
                Maybe::Some(Type::new(
                    TypeKind::Generic {
                        base: Heap::new(Type::new(
                            TypeKind::Path(verum_ast::Path::single(verum_ast::ty::Ident {
                                name: "List".to_string().into(),
                                span: Span::default(),
                            })),
                            Span::default(),
                        )),
                        args: List::from(vec![verum_ast::ty::GenericArg::Type(elem_type)]),
                    },
                    Span::default(),
                ))
            }

            // Result/Maybe types
            "result" | "res" => Maybe::Some(Type::new(
                TypeKind::Path(verum_ast::Path::single(verum_ast::ty::Ident {
                    name: "Result".to_string().into(),
                    span: Span::default(),
                })),
                Span::default(),
            )),

            "maybe" | "opt" | "option" => Maybe::Some(Type::new(
                TypeKind::Path(verum_ast::Path::single(verum_ast::ty::Ident {
                    name: "Maybe".to_string().into(),
                    span: Span::default(),
                })),
                Span::default(),
            )),

            // Default: use inferred type
            _ => Maybe::Some(Type::inferred(Span::default())),
        }
    }

    /// Infer expression type based on expression structure
    ///
    /// This performs bottom-up type inference by analyzing the expression structure.
    /// For fully accurate inference, this should integrate with the type checker,
    /// but for proof extraction we can derive many types directly from expression forms.
    ///
    /// Core type system: HM inference + refinement types + semantic types (List, Text, Map, etc.).
    fn infer_type(&self, expr: &Expr) -> Maybe<Type> {
        match &expr.kind {
            // Literal types are directly known
            ExprKind::Literal(lit) => Maybe::Some(self.literal_type(lit)),

            // Binary operators determine result type based on operation
            ExprKind::Binary { op, left, right } => {
                match op {
                    // Comparison operators always return Bool
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        Maybe::Some(Type::bool(expr.span))
                    }
                    // Logical operators return Bool
                    BinOp::And | BinOp::Or | BinOp::Imply => Maybe::Some(Type::bool(expr.span)),
                    // Arithmetic operators: infer from operands
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem | BinOp::Pow => {
                        // Try left operand first
                        if let Maybe::Some(left_ty) = self.infer_type(left) {
                            if !matches!(left_ty.kind, TypeKind::Inferred) {
                                return Maybe::Some(left_ty);
                            }
                        }
                        // Fall back to right operand
                        if let Maybe::Some(right_ty) = self.infer_type(right) {
                            if !matches!(right_ty.kind, TypeKind::Inferred) {
                                return Maybe::Some(right_ty);
                            }
                        }
                        // Default to Int for arithmetic
                        Maybe::Some(Type::int(expr.span))
                    }
                    // Bitwise operators return integer types
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                        Maybe::Some(Type::int(expr.span))
                    }
                    // Assignment operators return unit
                    _ if op.is_assignment() => Maybe::Some(Type::unit(expr.span)),
                    _ => Maybe::Some(Type::inferred(expr.span)),
                }
            }

            // Unary operators
            ExprKind::Unary { op, expr: inner } => match op {
                UnOp::Not => Maybe::Some(Type::bool(expr.span)),
                UnOp::Neg | UnOp::BitNot => self.infer_type(inner),
                UnOp::Deref => {
                    // Dereference removes one layer of reference
                    if let Maybe::Some(inner_ty) = self.infer_type(inner) {
                        match &inner_ty.kind {
                            TypeKind::Reference { inner, .. }
                            | TypeKind::CheckedReference { inner, .. }
                            | TypeKind::UnsafeReference { inner, .. } => {
                                Maybe::Some((**inner).clone())
                            }
                            _ => Maybe::Some(Type::inferred(expr.span)),
                        }
                    } else {
                        Maybe::Some(Type::inferred(expr.span))
                    }
                }
                UnOp::Ref | UnOp::RefChecked | UnOp::RefUnsafe => {
                    if let Maybe::Some(inner_ty) = self.infer_type(inner) {
                        Maybe::Some(Type::new(
                            TypeKind::Reference {
                                mutable: false,
                                inner: Heap::new(inner_ty),
                            },
                            expr.span,
                        ))
                    } else {
                        Maybe::Some(Type::inferred(expr.span))
                    }
                }
                UnOp::RefMut | UnOp::RefCheckedMut | UnOp::RefUnsafeMut => {
                    if let Maybe::Some(inner_ty) = self.infer_type(inner) {
                        Maybe::Some(Type::new(
                            TypeKind::Reference {
                                mutable: true,
                                inner: Heap::new(inner_ty),
                            },
                            expr.span,
                        ))
                    } else {
                        Maybe::Some(Type::inferred(expr.span))
                    }
                }
                _ => Maybe::Some(Type::inferred(expr.span)),
            },

            // If expressions: infer from branches
            ExprKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                // Try to infer from then branch
                if let Maybe::Some(ref then_expr) = then_branch.expr {
                    if let Maybe::Some(ty) = self.infer_type(then_expr) {
                        if !matches!(ty.kind, TypeKind::Inferred) {
                            return Maybe::Some(ty);
                        }
                    }
                }
                // Try else branch
                if let Maybe::Some(else_expr) = else_branch {
                    return self.infer_type(else_expr);
                }
                // Default to unit for if without else
                Maybe::Some(Type::unit(expr.span))
            }

            // Match expressions: infer from first arm
            ExprKind::Match { arms, .. } => {
                if let Some(first_arm) = arms.first() {
                    return self.infer_type(&first_arm.body);
                }
                Maybe::Some(Type::inferred(expr.span))
            }

            // Tuple expressions
            ExprKind::Tuple(elements) => {
                let mut elem_types = List::new();
                for elem in elements {
                    if let Maybe::Some(ty) = self.infer_type(elem) {
                        elem_types.push(ty);
                    } else {
                        elem_types.push(Type::inferred(expr.span));
                    }
                }
                Maybe::Some(Type::new(TypeKind::Tuple(elem_types), expr.span))
            }

            // Array expressions
            ExprKind::Array(array_expr) => match array_expr {
                verum_ast::ArrayExpr::List(elements) => {
                    let elem_type = if let Some(first) = elements.first() {
                        self.infer_type(first)
                            .unwrap_or_else(|| Type::inferred(expr.span))
                    } else {
                        Type::inferred(expr.span)
                    };
                    let size_expr = Expr::literal(Literal {
                        kind: LiteralKind::Int(verum_ast::literal::IntLit {
                            value: elements.len() as i128,
                            suffix: Maybe::None,
                        }),
                        span: expr.span,
                    });
                    Maybe::Some(Type::new(
                        TypeKind::Array {
                            element: Heap::new(elem_type),
                            size: Maybe::Some(Heap::new(size_expr)),
                        },
                        expr.span,
                    ))
                }
                verum_ast::ArrayExpr::Repeat { value, count: _ } => {
                    let elem_type = self
                        .infer_type(value)
                        .unwrap_or_else(|| Type::inferred(expr.span));
                    Maybe::Some(Type::new(
                        TypeKind::Array {
                            element: Heap::new(elem_type),
                            size: Maybe::None, // Size expression needs evaluation
                        },
                        expr.span,
                    ))
                }
            },

            // Block expressions: infer from final expression
            ExprKind::Block(block) => {
                if let Maybe::Some(ref final_expr) = block.expr {
                    return self.infer_type(final_expr);
                }
                Maybe::Some(Type::unit(expr.span))
            }

            // Closure expressions
            ExprKind::Closure {
                params,
                return_type,
                body,
                ..
            } => {
                // Use declared return type if available
                if let Maybe::Some(ret_ty) = return_type {
                    let mut param_types = List::new();
                    for param in params {
                        if let Maybe::Some(ref ty) = param.ty {
                            param_types.push(ty.clone());
                        } else {
                            param_types.push(Type::inferred(expr.span));
                        }
                    }
                    return Maybe::Some(Type::new(
                        TypeKind::Function {
                            params: param_types,
                            return_type: Heap::new(ret_ty.clone()),
                            calling_convention: Maybe::None,
                            contexts: ContextList::empty(),
                        },
                        expr.span,
                    ));
                }
                // Otherwise infer from body
                let ret_ty = self
                    .infer_type(body)
                    .unwrap_or_else(|| Type::inferred(expr.span));
                let mut param_types = List::new();
                for param in params {
                    if let Maybe::Some(ref ty) = param.ty {
                        param_types.push(ty.clone());
                    } else {
                        param_types.push(Type::inferred(expr.span));
                    }
                }
                Maybe::Some(Type::new(
                    TypeKind::Function {
                        params: param_types,
                        return_type: Heap::new(ret_ty),
                        calling_convention: Maybe::None,
                        contexts: ContextList::empty(),
                    },
                    expr.span,
                ))
            }

            // Range expressions
            ExprKind::Range { start, .. } => {
                // Infer element type from start
                let elem_type = if let Maybe::Some(s) = start {
                    self.infer_type(s).unwrap_or_else(|| Type::int(expr.span))
                } else {
                    Type::int(expr.span)
                };
                Maybe::Some(Type::new(
                    TypeKind::Generic {
                        base: Heap::new(Type::new(
                            TypeKind::Path(verum_ast::Path::single(verum_ast::ty::Ident {
                                name: "Range".to_string().into(),
                                span: Span::default(),
                            })),
                            Span::default(),
                        )),
                        args: List::from(vec![verum_ast::ty::GenericArg::Type(elem_type)]),
                    },
                    expr.span,
                ))
            }

            // Cast expressions: use the target type
            ExprKind::Cast { ty, .. } => Maybe::Some(ty.clone()),

            // Return with value
            ExprKind::Return(maybe_expr) => {
                if let Maybe::Some(ret_expr) = maybe_expr {
                    return self.infer_type(ret_expr);
                }
                Maybe::Some(Type::unit(expr.span))
            }

            // Try expression: unwrap Result
            ExprKind::Try(inner) => {
                if let Maybe::Some(inner_ty) = self.infer_type(inner) {
                    // Result<T, E> -> T on successful try
                    if let TypeKind::Generic { args, .. } = &inner_ty.kind {
                        if let Some(verum_ast::ty::GenericArg::Type(ok_ty)) = args.first() {
                            return Maybe::Some(ok_ty.clone());
                        }
                    }
                }
                Maybe::Some(Type::inferred(expr.span))
            }

            // Path expressions: would need symbol table lookup
            ExprKind::Path(_) => Maybe::Some(Type::inferred(expr.span)),

            // Default case
            _ => Maybe::Some(Type::inferred(expr.span)),
        }
    }

    /// Get type from literal
    fn literal_type(&self, lit: &Literal) -> Type {
        match &lit.kind {
            LiteralKind::Bool(_) => Type::bool(lit.span),
            LiteralKind::Int(int_lit) => {
                // Check for type suffix
                if let Some(ref suffix) = int_lit.suffix {
                    let type_name = match suffix {
                        verum_ast::literal::IntSuffix::I8 => "i8",
                        verum_ast::literal::IntSuffix::I16 => "i16",
                        verum_ast::literal::IntSuffix::I32 => "i32",
                        verum_ast::literal::IntSuffix::I64 => "i64",
                        verum_ast::literal::IntSuffix::I128 => "i128",
                        verum_ast::literal::IntSuffix::Isize => "isize",
                        verum_ast::literal::IntSuffix::U8 => "u8",
                        verum_ast::literal::IntSuffix::U16 => "u16",
                        verum_ast::literal::IntSuffix::U32 => "u32",
                        verum_ast::literal::IntSuffix::U64 => "u64",
                        verum_ast::literal::IntSuffix::U128 => "u128",
                        verum_ast::literal::IntSuffix::Usize => "usize",
                        verum_ast::literal::IntSuffix::Custom(_) => "Int",
                    };
                    Type::new(
                        TypeKind::Path(verum_ast::Path::single(verum_ast::ty::Ident {
                            name: type_name.to_string().into(),
                            span: lit.span,
                        })),
                        lit.span,
                    )
                } else {
                    Type::int(lit.span)
                }
            }
            LiteralKind::Float(float_lit) => {
                if let Some(ref suffix) = float_lit.suffix {
                    let type_name = match suffix {
                        verum_ast::literal::FloatSuffix::F32 => "f32",
                        verum_ast::literal::FloatSuffix::F64 => "f64",
                        verum_ast::literal::FloatSuffix::Custom(_) => "Float",
                    };
                    Type::new(
                        TypeKind::Path(verum_ast::Path::single(verum_ast::ty::Ident {
                            name: type_name.to_string().into(),
                            span: lit.span,
                        })),
                        lit.span,
                    )
                } else {
                    Type::float(lit.span)
                }
            }
            LiteralKind::Text(_) => Type::text(lit.span),
            LiteralKind::Char(_) => Type::new(TypeKind::Char, lit.span),
            _ => Type::inferred(lit.span),
        }
    }

    /// Extract property expression from proof
    #[allow(dead_code)] // Part of proof extraction API
    fn extract_property_from_proof(&self, proof: &ProofTerm) -> Maybe<Expr> {
        match proof {
            ProofTerm::Axiom { formula, .. } => Maybe::Some(formula.clone()),
            ProofTerm::Lemma { conclusion, .. } => Maybe::Some(conclusion.clone()),
            _ => Maybe::None,
        }
    }

    /// Build recursive function from induction proof
    ///
    /// This implements the Curry-Howard correspondence between:
    /// - Natural number induction: forall n. P(0) -> (forall k. P(k) -> P(k+1)) -> P(n)
    /// - Recursive functions: rec f(n) = if n == 0 then base else step(n, f(n-1))
    ///
    /// The transformation:
    /// 1. Base case proof P(0) becomes the base case return value
    /// 2. Inductive case proof P(k) -> P(k+1) becomes the recursive step
    /// 3. The induction variable becomes the function parameter
    ///
    /// For structural induction on natural numbers, we generate:
    /// ```verum
    /// fn extracted_recursive(n: Int) -> T {
    ///     if n == 0 {
    ///         base_expr
    ///     } else {
    ///         // In the inductive case, we have access to:
    ///         // - n: the current value
    ///         // - extracted_recursive(n - 1): the recursive result (IH)
    ///         let ih = extracted_recursive(n - 1);
    ///         inductive_expr[ih]
    ///     }
    /// }
    /// ```
    fn build_recursive_function(
        &self,
        param: &Text,
        base_expr: &Expr,
        inductive_expr: &Expr,
    ) -> Maybe<Expr> {
        use verum_ast::smallvec::SmallVec;

        // Create base case condition: param == 0
        let condition = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Heap::new(self.var_expr(param)),
                right: Heap::new(self.zero_expr()),
            },
            Span::default(),
        );

        let mut conditions = SmallVec::new();
        conditions.push(verum_ast::expr::ConditionKind::Expr(condition));

        // Build the recursive call for the inductive step
        // This creates: extracted_recursive(param - 1)
        let recursive_call = self.build_recursive_call(param);

        // Build the inductive step body which:
        // 1. Binds the induction hypothesis (recursive call result)
        // 2. Uses the inductive_expr with access to IH
        let inductive_body = self.build_inductive_body(param, &recursive_call, inductive_expr);

        Maybe::Some(Expr::new(
            ExprKind::If {
                condition: Heap::new(verum_ast::expr::IfCondition {
                    conditions,
                    span: Span::default(),
                }),
                then_branch: verum_ast::Block {
                    stmts: vec![].into(),
                    expr: Maybe::Some(Heap::new(base_expr.clone())),
                    span: Span::default(),
                },
                else_branch: Maybe::Some(Heap::new(inductive_body)),
            },
            Span::default(),
        ))
    }

    /// Build a recursive call expression: extracted_recursive(param - 1)
    fn build_recursive_call(&self, param: &Text) -> Expr {
        // Create param - 1
        let decremented = Expr::new(
            ExprKind::Binary {
                op: BinOp::Sub,
                left: Heap::new(self.var_expr(param)),
                right: Heap::new(self.one_expr()),
            },
            Span::default(),
        );

        // Create the recursive function call
        let func_path = verum_ast::Path::single(verum_ast::ty::Ident {
            name: "extracted_recursive".to_string().into(),
            span: Span::default(),
        });

        // Build the function call with the decremented argument
        // ExprKind::Call takes List<Expr> directly as args
        let args = List::from(vec![decremented]);

        Expr::new(
            ExprKind::Call {
                func: Heap::new(Expr::new(ExprKind::Path(func_path), Span::default())),
                type_args: List::new(),
                args,
            },
            Span::default(),
        )
    }

    /// Build the inductive body that binds the induction hypothesis
    ///
    /// Creates a block like:
    /// ```verum
    /// {
    ///     let ih = extracted_recursive(param - 1);
    ///     inductive_expr  // which can reference 'ih'
    /// }
    /// ```
    ///
    /// The inductive expression from the proof is transformed to use
    /// the bound 'ih' variable for the induction hypothesis.
    fn build_inductive_body(
        &self,
        param: &Text,
        recursive_call: &Expr,
        inductive_expr: &Expr,
    ) -> Expr {
        // Create the 'ih' binding pattern
        let ih_pattern = Pattern::ident(
            verum_ast::ty::Ident {
                name: "ih".to_string().into(),
                span: Span::default(),
            },
            false,
            Span::default(),
        );

        // Create let statement: let ih = recursive_call;
        let ih_binding = verum_ast::Stmt::let_stmt(
            ih_pattern,
            Maybe::None,
            Maybe::Some(recursive_call.clone()),
            Span::default(),
        );

        // Transform the inductive expression to properly reference:
        // 1. The induction hypothesis variable 'ih'
        // 2. The current parameter value 'param'
        let transformed_inductive = self.transform_inductive_expr(param, inductive_expr);

        // Create the block with binding and transformed expression
        Expr::new(
            ExprKind::Block(verum_ast::Block {
                stmts: vec![ih_binding].into(),
                expr: Maybe::Some(Heap::new(transformed_inductive)),
                span: Span::default(),
            }),
            Span::default(),
        )
    }

    /// Transform inductive expression to use proper variable references
    ///
    /// In the induction proof, the inductive hypothesis is typically referenced
    /// as a proof term. In the extracted program, we need to transform these
    /// references to use the 'ih' variable that holds the recursive result.
    fn transform_inductive_expr(&self, _param: &Text, inductive_expr: &Expr) -> Expr {
        // For simple cases, the inductive expression can be used directly
        // as it already contains the proper structure from the proof.
        //
        // In more complex cases, we would need to:
        // 1. Find references to the induction hypothesis proof term
        // 2. Replace them with references to the 'ih' variable
        // 3. Ensure proper variable scoping
        //
        // The proof term structure guides this transformation:
        // - Lambda abstractions introduce variable bindings
        // - Application of IH becomes 'ih' variable reference
        // - Other terms are preserved structurally

        // For now, return the inductive expression with the assumption
        // that the proof term to expression conversion has already
        // structured it appropriately for recursive use.
        inductive_expr.clone()
    }

    /// Create one literal (used for decrementing in recursion)
    fn one_expr(&self) -> Expr {
        Expr::literal(Literal {
            kind: LiteralKind::Int(verum_ast::literal::IntLit {
                value: 1,
                suffix: Maybe::None,
            }),
            span: Span::default(),
        })
    }

    /// Create variable reference expression
    fn var_expr(&self, name: &Text) -> Expr {
        Expr::new(
            ExprKind::Path(verum_ast::Path::single(verum_ast::ty::Ident {
                name: name.as_str().to_string().into(),
                span: Span::default(),
            })),
            Span::default(),
        )
    }

    /// Create zero literal
    fn zero_expr(&self) -> Expr {
        Expr::literal(Literal {
            kind: LiteralKind::Int(verum_ast::literal::IntLit {
                value: 0,
                suffix: Maybe::None,
            }),
            span: Span::default(),
        })
    }

    /// Create true literal
    #[allow(dead_code)] // Part of proof extraction API
    fn true_expr(&self) -> Expr {
        Expr::literal(Literal {
            kind: LiteralKind::Bool(true),
            span: Span::default(),
        })
    }

    /// Get Nat type
    fn nat_type(&self) -> Type {
        Type {
            kind: TypeKind::Path(verum_ast::Path::single(verum_ast::ty::Ident {
                name: "Nat".to_string().into(),
                span: Span::default(),
            })),
            span: Span::default(),
        }
    }

    /// Get unknown/inferred type
    #[allow(dead_code)] // Part of proof extraction API
    fn unknown_type(&self) -> Type {
        Type {
            kind: TypeKind::Path(verum_ast::Path::single(verum_ast::ty::Ident {
                name: "_".to_string().into(),
                span: Span::default(),
            })),
            span: Span::default(),
        }
    }

    /// Get extraction statistics
    pub fn stats(&self) -> &ExtractionStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = ExtractionStats::default();
    }
}

impl Default for ProgramExtractor {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Proof Irrelevance ====================

/// Proof irrelevance analyzer
///
/// Identifies and erases proof-irrelevant parts of terms.
/// Implements proof erasure for runtime efficiency.
///
/// Proof-irrelevant extraction: erase Prop-typed components, keeping only computational content.
pub struct ProofEraser {
    /// Statistics
    stats: ErasureStats,
}

impl ProofEraser {
    /// Create a new proof eraser
    pub fn new() -> Self {
        Self {
            stats: ErasureStats::default(),
        }
    }

    /// Erase proof-irrelevant parts from program
    ///
    /// Returns a new program with proofs erased, suitable for runtime execution.
    pub fn erase_proofs(&mut self, program: &ExtractedProgram) -> ExtractedProgram {
        self.stats.programs_processed += 1;

        let mut erased = program.clone();

        // Erase proofs in preconditions (keep only runtime checks)
        erased.preconditions = self.erase_proof_contracts(&program.preconditions);

        // Erase proofs in postconditions (keep only runtime checks)
        erased.postconditions = self.erase_proof_contracts(&program.postconditions);

        // Erase proofs in body
        erased.body = self.erase_expr_proofs(&program.body);

        // Remove source proof reference (no longer needed at runtime)
        if program.source_proof.is_some() {
            erased.source_proof = Maybe::None;
            self.stats.proofs_erased += 1;
        }

        erased
    }

    /// Erase proofs from contracts
    fn erase_proof_contracts(&mut self, contracts: &List<Contract>) -> List<Contract> {
        contracts
            .iter()
            .filter(|c| c.is_runtime) // Keep only runtime contracts
            .cloned()
            .collect()
    }

    /// Erase proofs from expression
    fn erase_expr_proofs(&mut self, expr: &Expr) -> Expr {
        // In Verum, most proof terms are already type-level and don't appear in runtime code
        // This would recursively traverse and erase any proof-specific constructs
        expr.clone()
    }

    /// Get erasure statistics
    pub fn stats(&self) -> &ErasureStats {
        &self.stats
    }
}

impl Default for ProofEraser {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Code Generation ====================

/// Code generator for extraction targets
///
/// Generates code in target language from extracted programs.
pub struct CodeGenerator {
    /// Target language
    target: ExtractionTarget,
}

impl CodeGenerator {
    /// Create a new code generator for target
    pub fn new(target: ExtractionTarget) -> Self {
        Self { target }
    }

    /// Generate code for extracted program
    pub fn generate(&self, program: &ExtractedProgram) -> Text {
        match self.target {
            ExtractionTarget::Verum => self.generate_verum(program),
            ExtractionTarget::OCaml => self.generate_ocaml(program),
            ExtractionTarget::Lean => self.generate_lean(program),
            ExtractionTarget::Coq => self.generate_coq(program),
        }
    }

    /// Generate Verum code
    fn generate_verum(&self, program: &ExtractedProgram) -> Text {
        let mut code = String::new();

        // Add documentation
        if let Maybe::Some(doc) = &program.documentation {
            code.push_str(&format!("/// {}\n", doc));
        }

        // Add attributes
        if program.is_extracted {
            code.push_str("@extracted\n");
        }

        // Function signature
        code.push_str("fn ");
        code.push_str(program.name.as_str());
        code.push('(');

        // Parameters
        for (i, param) in program.params.iter().enumerate() {
            if i > 0 {
                code.push_str(", ");
            }
            code.push_str(param.name.as_str());
            code.push_str(": ");
            code.push_str(&self.format_type(&param.ty));
        }

        code.push_str(") -> ");
        code.push_str(&self.format_type(&program.return_type));

        // Preconditions
        if !program.preconditions.is_empty() {
            code.push_str("\n    requires ");
            for (i, pre) in program.preconditions.iter().enumerate() {
                if i > 0 {
                    code.push_str(", ");
                }
                code.push_str(&self.format_expr(&pre.expr));
            }
        }

        // Postconditions
        if !program.postconditions.is_empty() {
            code.push_str("\n    ensures ");
            for (i, post) in program.postconditions.iter().enumerate() {
                if i > 0 {
                    code.push_str(", ");
                }
                code.push_str(&self.format_expr(&post.expr));
            }
        }

        code.push_str(" {\n    ");
        code.push_str(&self.format_expr(&program.body));
        code.push_str("\n}\n");

        code.into()
    }

    /// Generate OCaml code
    fn generate_ocaml(&self, program: &ExtractedProgram) -> Text {
        let mut code = String::new();

        code.push_str("let ");
        code.push_str(program.name.as_str());

        // Parameters
        for param in &program.params {
            code.push(' ');
            code.push_str(param.name.as_str());
        }

        code.push_str(" =\n  ");
        code.push_str(&self.format_expr_ocaml(&program.body));
        code.push('\n');

        code.into()
    }

    /// Generate Lean code
    fn generate_lean(&self, program: &ExtractedProgram) -> Text {
        let mut code = String::new();

        code.push_str("def ");
        code.push_str(program.name.as_str());

        // Parameters
        for param in &program.params {
            code.push_str(" (");
            code.push_str(param.name.as_str());
            code.push_str(" : ");
            code.push_str(&self.format_type(&param.ty));
            code.push(')');
        }

        code.push_str(" : ");
        code.push_str(&self.format_type(&program.return_type));
        code.push_str(" :=\n  ");
        code.push_str(&self.format_expr(&program.body));
        code.push('\n');

        code.into()
    }

    /// Generate Coq code
    fn generate_coq(&self, program: &ExtractedProgram) -> Text {
        let mut code = String::new();

        code.push_str("Definition ");
        code.push_str(program.name.as_str());

        // Parameters
        for param in &program.params {
            code.push_str(" (");
            code.push_str(param.name.as_str());
            code.push_str(" : ");
            code.push_str(&self.format_type(&param.ty));
            code.push(')');
        }

        code.push_str(" : ");
        code.push_str(&self.format_type(&program.return_type));
        code.push_str(" :=\n  ");
        code.push_str(&self.format_expr(&program.body));
        code.push_str(".\n");

        code.into()
    }

    /// Format type as Verum source code
    ///
    /// Handles all TypeKind variants to produce valid Verum type syntax.
    ///
    /// Type formatting: named types, generics, function types, tuples, refinement types.
    fn format_type(&self, ty: &Type) -> String {
        match &ty.kind {
            // Primitive types
            TypeKind::Unit => "()".to_string(),
            TypeKind::Bool => "Bool".to_string(),
            TypeKind::Int => "Int".to_string(),
            TypeKind::Float => "Float".to_string(),
            TypeKind::Char => "Char".to_string(),
            TypeKind::Text => "Text".to_string(),

            // Path types
            TypeKind::Path(path) => self.format_path(path),

            // Tuple types: (T1, T2, ...)
            TypeKind::Tuple(elements) => {
                if elements.is_empty() {
                    "()".to_string()
                } else {
                    let elem_strs: Vec<String> =
                        elements.iter().map(|t| self.format_type(t)).collect();
                    format!("({})", elem_strs.join(", "))
                }
            }

            // Array types: [T; N]
            TypeKind::Array { element, size } => {
                let elem_str = self.format_type(element);
                if let Maybe::Some(size_expr) = size {
                    format!("[{}; {}]", elem_str, self.format_expr(size_expr))
                } else {
                    format!("[{}]", elem_str)
                }
            }

            // Slice types: [T]
            TypeKind::Slice(inner) => format!("[{}]", self.format_type(inner)),

            // Function types: fn(A, B) -> C
            TypeKind::Function {
                params,
                return_type,
                ..
            }
            | TypeKind::Rank2Function {
                type_params: _,
                params,
                return_type,
                ..
            } => {
                let param_strs: Vec<String> = params.iter().map(|t| self.format_type(t)).collect();
                format!(
                    "fn({}) -> {}",
                    param_strs.join(", "),
                    self.format_type(return_type)
                )
            }

            // Reference types
            TypeKind::Reference { mutable, inner } => {
                if *mutable {
                    format!("&mut {}", self.format_type(inner))
                } else {
                    format!("&{}", self.format_type(inner))
                }
            }

            // Checked reference types
            TypeKind::CheckedReference { mutable, inner } => {
                if *mutable {
                    format!("&checked mut {}", self.format_type(inner))
                } else {
                    format!("&checked {}", self.format_type(inner))
                }
            }

            // Unsafe reference types
            TypeKind::UnsafeReference { mutable, inner } => {
                if *mutable {
                    format!("&unsafe mut {}", self.format_type(inner))
                } else {
                    format!("&unsafe {}", self.format_type(inner))
                }
            }

            // Pointer types
            TypeKind::Pointer { mutable, inner } => {
                if *mutable {
                    format!("*mut {}", self.format_type(inner))
                } else {
                    format!("*const {}", self.format_type(inner))
                }
            }

            // Volatile pointer types for MMIO
            TypeKind::VolatilePointer { mutable, inner } => {
                if *mutable {
                    format!("*volatile mut {}", self.format_type(inner))
                } else {
                    format!("*volatile {}", self.format_type(inner))
                }
            }

            // Generic types: List<T>
            TypeKind::Generic { base, args } => {
                let base_str = self.format_type(base);
                let arg_strs: Vec<String> = args
                    .iter()
                    .map(|arg| match arg {
                        verum_ast::ty::GenericArg::Type(t) => self.format_type(t),
                        verum_ast::ty::GenericArg::Const(e) => self.format_expr(e),
                        verum_ast::ty::GenericArg::Lifetime(lt) => format!("'{}", lt.name),
                        verum_ast::ty::GenericArg::Binding(binding) => {
                            format!("{} = {}", binding.name.name, self.format_type(&binding.ty))
                        }
                    })
                    .collect();
                format!("{}<{}>", base_str, arg_strs.join(", "))
            }

            // Qualified types: <T as Protocol>::AssocType
            TypeKind::Qualified {
                self_ty,
                trait_ref,
                assoc_name,
            } => {
                format!(
                    "<{} as {}>::{}",
                    self.format_type(self_ty),
                    self.format_path(trait_ref),
                    assoc_name.name
                )
            }

            // Refinement types: T{predicate}
            TypeKind::Refined { base, predicate } => {
                let base_str = self.format_type(base);
                let pred_str = self.format_expr(&predicate.expr);
                if let Maybe::Some(ref binding) = predicate.binding {
                    format!("{}{{|{}| {}}}", base_str, binding.name, pred_str)
                } else {
                    format!("{}{{{}}}", base_str, pred_str)
                }
            }

            // Sigma types: x: T where predicate
            TypeKind::Sigma {
                name,
                base,
                predicate,
            } => {
                format!(
                    "{}: {} where {}",
                    name.name,
                    self.format_type(base),
                    self.format_expr(predicate)
                )
            }

            // Inferred type
            TypeKind::Inferred => "_".to_string(),

            // Bounded types: T where T: Protocol
            TypeKind::Bounded { base, bounds } => {
                let base_str = self.format_type(base);
                let bound_strs: Vec<String> = bounds
                    .iter()
                    .map(|b| match &b.kind {
                        verum_ast::ty::TypeBoundKind::Protocol(path) => self.format_path(path),
                        verum_ast::ty::TypeBoundKind::Equality(ty) => {
                            format!("= {}", self.format_type(ty))
                        }
                        verum_ast::ty::TypeBoundKind::NegativeProtocol(path) => {
                            format!("!{}", self.format_path(path))
                        }
                        verum_ast::ty::TypeBoundKind::AssociatedTypeBound {
                            type_path,
                            assoc_name,
                            bounds,
                        } => {
                            let bound_strs: Vec<String> = bounds
                                .iter()
                                .map(|b| self.format_type_bound(b))
                                .collect();
                            format!(
                                "{}.{}: {}",
                                self.format_path(type_path),
                                assoc_name.name,
                                bound_strs.join(" + ")
                            )
                        }
                        verum_ast::ty::TypeBoundKind::AssociatedTypeEquality {
                            type_path,
                            assoc_name,
                            eq_type,
                        } => {
                            format!(
                                "{}.{} = {}",
                                self.format_path(type_path),
                                assoc_name.name,
                                self.format_type(eq_type)
                            )
                        }
                        verum_ast::ty::TypeBoundKind::GenericProtocol(ty) => {
                            self.format_type(ty)
                        }
                    })
                    .collect();
                if bounds.is_empty() {
                    base_str
                } else {
                    format!(
                        "{} where {}: {}",
                        base_str,
                        base_str,
                        bound_strs.join(" + ")
                    )
                }
            }

            // Dynamic protocol types: dyn Display + Debug
            TypeKind::DynProtocol { bounds, bindings } => {
                let bound_strs: Vec<String> = bounds
                    .iter()
                    .map(|b| match &b.kind {
                        verum_ast::ty::TypeBoundKind::Protocol(path) => self.format_path(path),
                        _ => "_".to_string(),
                    })
                    .collect();
                let mut result = format!("dyn {}", bound_strs.join(" + "));
                if let Maybe::Some(binds) = bindings {
                    let bind_strs: Vec<String> = binds
                        .iter()
                        .map(|b| format!("{} = {}", b.name.name, self.format_type(&b.ty)))
                        .collect();
                    result.push_str(&format!("<{}>", bind_strs.join(", ")));
                }
                result
            }

            // Ownership types: %T
            TypeKind::Ownership { mutable, inner } => {
                if *mutable {
                    format!("%mut {}", self.format_type(inner))
                } else {
                    format!("%{}", self.format_type(inner))
                }
            }

            // GenRef types
            TypeKind::GenRef { inner } => format!("GenRef<{}>", self.format_type(inner)),

            // Type constructor: F<_>
            TypeKind::TypeConstructor { base, arity } => {
                let placeholders: Vec<&str> = (0..*arity).map(|_| "_").collect();
                format!("{}<{}>", self.format_type(base), placeholders.join(", "))
            }

            // Tensor types
            TypeKind::Tensor {
                element,
                shape,
                layout,
            } => {
                let elem_str = self.format_type(element);
                let shape_strs: Vec<String> = shape.iter().map(|e| self.format_expr(e)).collect();
                let layout_str = match layout {
                    Maybe::Some(verum_ast::ty::TensorLayout::RowMajor) => ", row_major",
                    Maybe::Some(verum_ast::ty::TensorLayout::ColumnMajor) => ", column_major",
                    Maybe::None => "",
                };
                format!(
                    "Tensor<{}, [{}]{}> ",
                    elem_str,
                    shape_strs.join(", "),
                    layout_str
                )
            }

            // Existential types: some T: Bound
            TypeKind::Existential { name, bounds } => {
                let bound_strs: Vec<String> = bounds
                    .iter()
                    .map(|b| self.format_type_bound(b))
                    .collect();
                if bounds.is_empty() {
                    format!("some {}", name.name)
                } else {
                    format!("some {}: {}", name.name, bound_strs.join(" + "))
                }
            }

            // Associated type paths: T.Item
            TypeKind::AssociatedType { base, assoc } => {
                format!("{}.{}", self.format_type(base), assoc.name)
            }

            // Never type (!) - diverging expressions
            TypeKind::Never => "!".to_string(),

            // Capability-restricted type: T with [Cap1, Cap2, ...]
            TypeKind::CapabilityRestricted { base, capabilities } => {
                let base_str = self.format_type(base);
                let cap_strs: Vec<String> = capabilities
                    .capabilities
                    .iter()
                    .map(|c| c.as_str().to_string())
                    .collect();
                format!("{} with [{}]", base_str, cap_strs.join(", "))
            }

            // Unknown type (top type)
            TypeKind::Unknown => "unknown".to_string(),

            // Record types: { field1: Type1, field2: Type2, ... }
            TypeKind::Record { fields } => {
                let field_strs: Vec<String> = fields
                    .iter()
                    .map(|f| format!("{}: {}", f.name.name, self.format_type(&f.ty)))
                    .collect();
                format!("{{ {} }}", field_strs.join(", "))
            }

            // Universe types: Type, Type(0), Type(1), Type(u)
            TypeKind::Universe { level } => {
                match level {
                    verum_common::Maybe::None => "Type".to_string(),
                    verum_common::Maybe::Some(verum_ast::UniverseLevelExpr::Concrete(n)) => {
                        format!("Type({})", n)
                    }
                    verum_common::Maybe::Some(verum_ast::UniverseLevelExpr::Variable(ident)) => {
                        format!("Type({})", ident.name)
                    }
                    verum_common::Maybe::Some(_) => "Type".to_string(),
                }
            }

            // Meta types: meta T
            TypeKind::Meta { inner } => {
                format!("meta {}", self.format_type(inner))
            }

            // Type lambdas: |x| T
            TypeKind::TypeLambda { params, body } => {
                let param_strs: Vec<String> = params.iter().map(|p| p.name.to_string()).collect();
                format!("|{}| {}", param_strs.join(", "), self.format_type(body))
            }

            // Path equality type: Path<A>(lhs, rhs)
            TypeKind::PathType { carrier, lhs, rhs } => {
                format!(
                    "Path<{}>({})",
                    self.format_type(carrier),
                    format!(
                        "{}, {}",
                        self.format_expr(lhs),
                        self.format_expr(rhs)
                    )
                )
            }
        }
    }

    /// Format path as string
    fn format_path(&self, path: &verum_ast::Path) -> String {
        path.segments
            .iter()
            .map(|seg| match seg {
                verum_ast::ty::PathSegment::Name(ident) => ident.name.to_string(),
                verum_ast::ty::PathSegment::SelfValue => "self".to_string(),
                verum_ast::ty::PathSegment::Super => "super".to_string(),
                verum_ast::ty::PathSegment::Cog => "cog".to_string(),
                verum_ast::ty::PathSegment::Relative => ".".to_string(),
            })
            .collect::<Vec<_>>()
            .join("::")
    }

    /// Format a type bound as a string
    fn format_type_bound(&self, bound: &verum_ast::ty::TypeBound) -> String {
        match &bound.kind {
            verum_ast::ty::TypeBoundKind::Protocol(path) => self.format_path(path),
            verum_ast::ty::TypeBoundKind::Equality(ty) => {
                format!("= {}", self.format_type(ty))
            }
            verum_ast::ty::TypeBoundKind::NegativeProtocol(path) => {
                format!("!{}", self.format_path(path))
            }
            verum_ast::ty::TypeBoundKind::AssociatedTypeBound {
                type_path,
                assoc_name,
                bounds,
            } => {
                let bound_strs: Vec<String> = bounds
                    .iter()
                    .map(|b| self.format_type_bound(b))
                    .collect();
                format!(
                    "{}.{}: {}",
                    self.format_path(type_path),
                    assoc_name.name,
                    bound_strs.join(" + ")
                )
            }
            verum_ast::ty::TypeBoundKind::AssociatedTypeEquality {
                type_path,
                assoc_name,
                eq_type,
            } => {
                format!(
                    "{}.{} = {}",
                    self.format_path(type_path),
                    assoc_name.name,
                    self.format_type(eq_type)
                )
            }
            verum_ast::ty::TypeBoundKind::GenericProtocol(ty) => self.format_type(ty),
        }
    }

    /// Format expression as Verum source code
    ///
    /// Handles all ExprKind variants to produce valid Verum expression syntax.
    ///
    /// Expression formatting: literals, operators, calls, match, if-else, closures.
    fn format_expr(&self, expr: &Expr) -> String {
        match &expr.kind {
            // Literals
            ExprKind::Literal(lit) => self.format_literal(lit),

            // Paths
            ExprKind::Path(path) => self.format_path(path),

            // Binary operations
            ExprKind::Binary { op, left, right } => {
                format!(
                    "({} {} {})",
                    self.format_expr(left),
                    op.as_str(),
                    self.format_expr(right)
                )
            }

            // Unary operations
            ExprKind::Unary { op, expr: inner } => {
                format!("{}{}", op.as_str(), self.format_expr(inner))
            }

            // Function calls
            ExprKind::Call { func, args, .. } => {
                let arg_strs: Vec<String> = args.iter().map(|a| self.format_expr(a)).collect();
                format!("{}({})", self.format_expr(func), arg_strs.join(", "))
            }

            // Method calls
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                let arg_strs: Vec<String> = args.iter().map(|a| self.format_expr(a)).collect();
                format!(
                    "{}.{}({})",
                    self.format_expr(receiver),
                    method.name,
                    arg_strs.join(", ")
                )
            }

            // Field access
            ExprKind::Field { expr: inner, field } => {
                format!("{}.{}", self.format_expr(inner), field.name)
            }

            // Optional chaining
            ExprKind::OptionalChain { expr: inner, field } => {
                format!("{}?.{}", self.format_expr(inner), field.name)
            }

            // Tuple indexing
            ExprKind::TupleIndex { expr: inner, index } => {
                format!("{}.{}", self.format_expr(inner), index)
            }

            // Indexing
            ExprKind::Index { expr: inner, index } => {
                format!("{}[{}]", self.format_expr(inner), self.format_expr(index))
            }

            // Pipeline
            ExprKind::Pipeline { left, right } => {
                format!("{} |> {}", self.format_expr(left), self.format_expr(right))
            }

            // Null coalescing
            ExprKind::NullCoalesce { left, right } => {
                format!("{} ?? {}", self.format_expr(left), self.format_expr(right))
            }

            // Type cast
            ExprKind::Cast { expr: inner, ty } => {
                format!("{} as {}", self.format_expr(inner), self.format_type(ty))
            }

            // Try expression
            ExprKind::Try(inner) => format!("{}?", self.format_expr(inner)),

            // Tuples
            ExprKind::Tuple(elements) => {
                let elem_strs: Vec<String> = elements.iter().map(|e| self.format_expr(e)).collect();
                format!("({})", elem_strs.join(", "))
            }

            // Arrays
            ExprKind::Array(array_expr) => match array_expr {
                verum_ast::ArrayExpr::List(elements) => {
                    let elem_strs: Vec<String> =
                        elements.iter().map(|e| self.format_expr(e)).collect();
                    format!("[{}]", elem_strs.join(", "))
                }
                verum_ast::ArrayExpr::Repeat { value, count } => {
                    format!("[{}; {}]", self.format_expr(value), self.format_expr(count))
                }
            },

            // List comprehensions
            ExprKind::Comprehension {
                expr: inner,
                clauses,
            } => {
                let clause_strs: Vec<String> = clauses
                    .iter()
                    .map(|c| self.format_comprehension_clause(c))
                    .collect();
                format!("[{} {}]", self.format_expr(inner), clause_strs.join(" "))
            }

            // Records
            ExprKind::Record { path, fields, base } => {
                let field_strs: Vec<String> = fields
                    .iter()
                    .map(|f| {
                        if let Maybe::Some(ref value) = f.value {
                            format!("{}: {}", f.name.name, self.format_expr(value))
                        } else {
                            f.name.name.to_string()
                        }
                    })
                    .collect();
                let base_str = if let Maybe::Some(b) = base {
                    format!(", ..{}", self.format_expr(b))
                } else {
                    String::new()
                };
                format!(
                    "{} {{ {}{} }}",
                    self.format_path(path),
                    field_strs.join(", "),
                    base_str
                )
            }

            // Interpolated strings
            ExprKind::InterpolatedString {
                handler,
                parts,
                exprs,
            } => {
                let mut result = format!("{}\"", handler);
                for (i, part) in parts.iter().enumerate() {
                    result.push_str(part.as_str());
                    if i < exprs.len() {
                        result.push_str(&format!("{{{}}}", self.format_expr(&exprs[i])));
                    }
                }
                result.push('"');
                result
            }

            // Blocks
            ExprKind::Block(block) => {
                let mut lines = Vec::new();
                for stmt in &block.stmts {
                    lines.push(self.format_stmt(stmt));
                }
                if let Maybe::Some(ref final_expr) = block.expr {
                    lines.push(self.format_expr(final_expr));
                }
                format!("{{ {} }}", lines.join("; "))
            }

            // If expressions
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_str = self.format_if_condition(condition);
                let then_str = self.format_block(then_branch);
                if let Maybe::Some(else_expr) = else_branch {
                    format!(
                        "if {} {} else {}",
                        cond_str,
                        then_str,
                        self.format_expr(else_expr)
                    )
                } else {
                    format!("if {} {}", cond_str, then_str)
                }
            }

            // Match expressions
            ExprKind::Match { expr: inner, arms } => {
                let arm_strs: Vec<String> = arms
                    .iter()
                    .map(|arm| {
                        let pat_str = self.format_pattern(&arm.pattern);
                        let guard_str = if let Maybe::Some(ref g) = arm.guard {
                            format!(" if {}", self.format_expr(g))
                        } else {
                            String::new()
                        };
                        format!(
                            "{}{} => {}",
                            pat_str,
                            guard_str,
                            self.format_expr(&arm.body)
                        )
                    })
                    .collect();
                format!(
                    "match {} {{ {} }}",
                    self.format_expr(inner),
                    arm_strs.join(", ")
                )
            }

            // Loops
            ExprKind::Loop { label, body, .. } => {
                let label_str = if let Maybe::Some(l) = label {
                    format!("'{}: ", l)
                } else {
                    String::new()
                };
                format!("{}loop {}", label_str, self.format_block(body))
            }

            ExprKind::While {
                label,
                condition,
                body,
                ..
            } => {
                let label_str = if let Maybe::Some(l) = label {
                    format!("'{}: ", l)
                } else {
                    String::new()
                };
                format!(
                    "{}while {} {}",
                    label_str,
                    self.format_expr(condition),
                    self.format_block(body)
                )
            }

            ExprKind::For {
                label,
                pattern,
                iter,
                body,
                ..
            } => {
                let label_str = if let Maybe::Some(l) = label {
                    format!("'{}: ", l)
                } else {
                    String::new()
                };
                format!(
                    "{}for {} in {} {}",
                    label_str,
                    self.format_pattern(pattern),
                    self.format_expr(iter),
                    self.format_block(body)
                )
            }

            // Control flow
            ExprKind::Break { label, value } => {
                let label_str = if let Maybe::Some(l) = label {
                    format!(" '{}", l)
                } else {
                    String::new()
                };
                let value_str = if let Maybe::Some(v) = value {
                    format!(" {}", self.format_expr(v))
                } else {
                    String::new()
                };
                format!("break{}{}", label_str, value_str)
            }

            ExprKind::Continue { label } => {
                if let Maybe::Some(l) = label {
                    format!("continue '{}", l)
                } else {
                    "continue".to_string()
                }
            }

            ExprKind::Return(value) => {
                if let Maybe::Some(v) = value {
                    format!("return {}", self.format_expr(v))
                } else {
                    "return".to_string()
                }
            }

            ExprKind::Yield(value) => format!("yield {}", self.format_expr(value)),

            // Closures
            ExprKind::Closure {
                async_,
                move_,
                params,
                return_type,
                body,
                ..
            } => {
                let async_str = if *async_ { "async " } else { "" };
                let move_str = if *move_ { "move " } else { "" };
                let param_strs: Vec<String> = params
                    .iter()
                    .map(|p| {
                        let pat_str = self.format_pattern(&p.pattern);
                        if let Maybe::Some(ref ty) = p.ty {
                            format!("{}: {}", pat_str, self.format_type(ty))
                        } else {
                            pat_str
                        }
                    })
                    .collect();
                let ret_str = if let Maybe::Some(ty) = return_type {
                    format!(" -> {}", self.format_type(ty))
                } else {
                    String::new()
                };
                format!(
                    "{}{}|{}|{} {}",
                    async_str,
                    move_str,
                    param_strs.join(", "),
                    ret_str,
                    self.format_expr(body)
                )
            }

            // Async/await
            ExprKind::Async(block) => format!("async {}", self.format_block(block)),
            ExprKind::Await(inner) => format!("{}.await", self.format_expr(inner)),
            ExprKind::Spawn { expr: inner, .. } => format!("spawn {}", self.format_expr(inner)),

            // Unsafe and meta
            ExprKind::Unsafe(block) => format!("unsafe {}", self.format_block(block)),
            ExprKind::Meta(block) => format!("meta {}", self.format_block(block)),

            // Range expressions
            ExprKind::Range {
                start,
                end,
                inclusive,
            } => {
                let start_str = start
                    .as_ref()
                    .map(|s| self.format_expr(s))
                    .unwrap_or_default();
                let end_str = end
                    .as_ref()
                    .map(|e| self.format_expr(e))
                    .unwrap_or_default();
                let op = if *inclusive { "..=" } else { ".." };
                format!("{}{}{}", start_str, op, end_str)
            }

            // Quantifiers
            ExprKind::Forall { bindings, body } => {
                let bindings_str = self.format_quantifier_bindings(bindings);
                format!("forall {}. {}", bindings_str, self.format_expr(body))
            }

            ExprKind::Exists { bindings, body } => {
                let bindings_str = self.format_quantifier_bindings(bindings);
                format!("exists {}. {}", bindings_str, self.format_expr(body))
            }

            // Parenthesized
            ExprKind::Paren(inner) => format!("({})", self.format_expr(inner)),

            // Try-recover patterns
            ExprKind::TryRecover { try_block, recover } => {
                let recover_str = match recover {
                    RecoverBody::MatchArms { arms, .. } => {
                        let arm_strs: Vec<String> = arms
                            .iter()
                            .map(|arm| {
                                format!(
                                    "{} => {}",
                                    self.format_pattern(&arm.pattern),
                                    self.format_expr(&arm.body)
                                )
                            })
                            .collect();
                        format!("{{ {} }}", arm_strs.join(", "))
                    }
                    RecoverBody::Closure { param, body, .. } => {
                        format!("|{}| {}", self.format_pattern(&param.pattern), self.format_expr(body))
                    }
                };
                format!(
                    "try {} recover {}",
                    self.format_expr(try_block),
                    recover_str
                )
            }

            ExprKind::TryFinally {
                try_block,
                finally_block,
            } => {
                format!(
                    "try {} finally {}",
                    self.format_expr(try_block),
                    self.format_expr(finally_block)
                )
            }

            ExprKind::TryRecoverFinally {
                try_block,
                recover,
                finally_block,
            } => {
                let recover_str = match recover {
                    RecoverBody::MatchArms { arms, .. } => {
                        let arm_strs: Vec<String> = arms
                            .iter()
                            .map(|arm| {
                                format!(
                                    "{} => {}",
                                    self.format_pattern(&arm.pattern),
                                    self.format_expr(&arm.body)
                                )
                            })
                            .collect();
                        format!("{{ {} }}", arm_strs.join(", "))
                    }
                    RecoverBody::Closure { param, body, .. } => {
                        format!("|{}| {}", self.format_pattern(&param.pattern), self.format_expr(body))
                    }
                };
                format!(
                    "try {} recover {} finally {}",
                    self.format_expr(try_block),
                    recover_str,
                    self.format_expr(finally_block)
                )
            }

            // Other expression kinds get generic representation
            _ => format!(
                "/* unformatted: {:?} */",
                std::mem::discriminant(&expr.kind)
            ),
        }
    }

    /// Format literal value
    fn format_literal(&self, lit: &Literal) -> String {
        match &lit.kind {
            LiteralKind::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            LiteralKind::Int(int_lit) => {
                let value_str = int_lit.value.to_string();
                if let Some(ref suffix) = int_lit.suffix {
                    format!("{}{}", value_str, suffix.as_str())
                } else {
                    value_str
                }
            }
            LiteralKind::Float(float_lit) => {
                let value_str = float_lit.value.to_string();
                if let Some(ref suffix) = float_lit.suffix {
                    format!("{}{}", value_str, suffix.as_str())
                } else {
                    value_str
                }
            }
            LiteralKind::Text(string_lit) => {
                match string_lit {
                    verum_ast::literal::StringLit::Regular(s) => format!("\"{}\"", s),
                    verum_ast::literal::StringLit::MultiLine(s) => format!("\"\"\"{}\"\"\"", s),
                }
            }
            LiteralKind::Char(c) => format!("'{}'", c),
            LiteralKind::ByteChar(b) => format!("b'{}'", *b as char),
            LiteralKind::ByteString(bytes) => {
                let escaped: String = bytes.iter().map(|b| format!("\\x{:02x}", b)).collect();
                format!("b\"{}\"", escaped)
            }
            LiteralKind::Tagged { tag, content } => format!("{}#\"{}\"", tag, content),
            LiteralKind::InterpolatedString(interp) => {
                format!("{}\"{}\"", interp.prefix, interp.content)
            }
            LiteralKind::Contract(content) => format!("contract#\"{}\"", content),
            LiteralKind::Composite(comp) => {
                format!("{}#{}", comp.tag, comp.delimiter.wrap(&comp.content))
            }
            LiteralKind::ContextAdaptive(ctx_lit) => ctx_lit.raw.to_string(),
        }
    }

    /// Format quantifier bindings for forall/exists expressions
    fn format_quantifier_bindings(&self, bindings: &[verum_ast::expr::QuantifierBinding]) -> String {
        bindings
            .iter()
            .map(|binding| {
                let mut parts = vec![self.format_pattern(&binding.pattern)];
                if let verum_common::Maybe::Some(ty) = &binding.ty {
                    parts.push(format!(": {}", self.format_type(ty)));
                }
                if let verum_common::Maybe::Some(domain) = &binding.domain {
                    parts.push(format!(" in {}", self.format_expr(domain)));
                }
                if let verum_common::Maybe::Some(guard) = &binding.guard {
                    parts.push(format!(" where {}", self.format_expr(guard)));
                }
                parts.join("")
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Format pattern
    fn format_pattern(&self, pattern: &Pattern) -> String {
        match &pattern.kind {
            PatternKind::Wildcard => "_".to_string(),
            PatternKind::Rest => "..".to_string(),
            PatternKind::Ident {
                by_ref,
                mutable,
                name,
                subpattern,
            } => {
                let ref_str = if *by_ref { "ref " } else { "" };
                let mut_str = if *mutable { "mut " } else { "" };
                let sub_str = if let Maybe::Some(sub) = subpattern {
                    format!(" @ {}", self.format_pattern(sub))
                } else {
                    String::new()
                };
                format!("{}{}{}{}", ref_str, mut_str, name.name, sub_str)
            }
            PatternKind::Literal(lit) => self.format_literal(lit),
            PatternKind::Tuple(elements) => {
                let elem_strs: Vec<String> =
                    elements.iter().map(|p| self.format_pattern(p)).collect();
                format!("({})", elem_strs.join(", "))
            }
            PatternKind::Array(elements) => {
                let elem_strs: Vec<String> =
                    elements.iter().map(|p| self.format_pattern(p)).collect();
                format!("[{}]", elem_strs.join(", "))
            }
            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                let mut parts = Vec::new();
                for p in before {
                    parts.push(self.format_pattern(p));
                }
                if let Maybe::Some(r) = rest {
                    parts.push(format!("..{}", self.format_pattern(r)));
                } else {
                    parts.push("..".to_string());
                }
                for p in after {
                    parts.push(self.format_pattern(p));
                }
                format!("[{}]", parts.join(", "))
            }
            PatternKind::Record { path, fields, rest } => {
                let field_strs: Vec<String> = fields
                    .iter()
                    .map(|f| {
                        if let Maybe::Some(ref pat) = f.pattern {
                            format!("{}: {}", f.name.name, self.format_pattern(pat))
                        } else {
                            f.name.name.to_string()
                        }
                    })
                    .collect();
                let rest_str = if *rest { ", .." } else { "" };
                format!(
                    "{} {{ {}{} }}",
                    self.format_path(path),
                    field_strs.join(", "),
                    rest_str
                )
            }
            PatternKind::Variant { path, data } => {
                let path_str = self.format_path(path);
                match data {
                    Maybe::Some(verum_ast::pattern::VariantPatternData::Tuple(patterns)) => {
                        let pat_strs: Vec<String> =
                            patterns.iter().map(|p| self.format_pattern(p)).collect();
                        format!("{}({})", path_str, pat_strs.join(", "))
                    }
                    Maybe::Some(verum_ast::pattern::VariantPatternData::Record {
                        fields,
                        rest,
                    }) => {
                        let field_strs: Vec<String> = fields
                            .iter()
                            .map(|f| {
                                if let Maybe::Some(ref pat) = f.pattern {
                                    format!("{}: {}", f.name.name, self.format_pattern(pat))
                                } else {
                                    f.name.name.to_string()
                                }
                            })
                            .collect();
                        let rest_str = if *rest { ", .." } else { "" };
                        format!("{} {{ {}{} }}", path_str, field_strs.join(", "), rest_str)
                    }
                    Maybe::None => path_str,
                }
            }
            PatternKind::Or(alternatives) => {
                let alt_strs: Vec<String> = alternatives
                    .iter()
                    .map(|p| self.format_pattern(p))
                    .collect();
                alt_strs.join(" | ")
            }
            PatternKind::Reference { mutable, inner } => {
                if *mutable {
                    format!("&mut {}", self.format_pattern(inner))
                } else {
                    format!("&{}", self.format_pattern(inner))
                }
            }
            PatternKind::Range {
                start,
                end,
                inclusive,
            } => {
                let start_str = start
                    .as_ref()
                    .map(|s| self.format_literal(s))
                    .unwrap_or_default();
                let end_str = end
                    .as_ref()
                    .map(|e| self.format_literal(e))
                    .unwrap_or_default();
                let op = if *inclusive { "..=" } else { ".." };
                format!("{}{}{}", start_str, op, end_str)
            }
            PatternKind::Paren(inner) => format!("({})", self.format_pattern(inner)),            PatternKind::View {
                view_function,
                pattern,
            } => {
                format!(
                    "{} -> {}",
                    self.format_expr(view_function),
                    self.format_pattern(pattern)
                )
            }
            PatternKind::Active { name, params, bindings } => {
                let mut result = String::new();
                result.push_str(&name.name);
                if !params.is_empty() {
                    let args: Vec<String> = params.iter().map(|e| self.format_expr(e)).collect();
                    result.push_str(&format!("({})", args.join(", ")));
                }
                result.push('(');
                if !bindings.is_empty() {
                    let binding_strs: Vec<String> = bindings.iter().map(|p| self.format_pattern(p)).collect();
                    result.push_str(&binding_strs.join(", "));
                }
                result.push(')');
                result
            }
            PatternKind::And(patterns) => {
                let pat_strs: Vec<String> = patterns.iter().map(|p| self.format_pattern(p)).collect();
                pat_strs.join(" & ")
            }
            PatternKind::TypeTest { binding, test_type } => {
                format!("{} is {}", binding.name, self.format_type(test_type))
            }
            // Stream pattern: stream[first, second, ...rest]
            // Stream pattern matching: destructure stream into head elements and rest
            PatternKind::Stream { head_patterns, rest } => {
                let head_strs: Vec<String> = head_patterns.iter().map(|p| self.format_pattern(p)).collect();
                let rest_str = if let Maybe::Some(rest_ident) = rest {
                    if head_strs.is_empty() {
                        format!("...{}", rest_ident.name)
                    } else {
                        format!(", ...{}", rest_ident.name)
                    }
                } else {
                    String::new()
                };
                format!("stream[{}{}]", head_strs.join(", "), rest_str)
            }
            // Cons pattern: head :: tail
            PatternKind::Cons { head, tail } => {
                format!("{} :: {}", self.format_pattern(head), self.format_pattern(tail))
            }
            // Guard pattern: pattern if guard_expr
            // Spec: Rust RFC 3637 - Guard Patterns
            PatternKind::Guard { pattern, guard } => {
                format!("{} if {}", self.format_pattern(pattern), self.format_expr(guard))
            }
        }
    }

    /// Format statement
    fn format_stmt(&self, stmt: &verum_ast::Stmt) -> String {
        match &stmt.kind {
            verum_ast::stmt::StmtKind::Let {
                pattern, ty, value, ..
            } => {
                let pat_str = self.format_pattern(pattern);
                let ty_str = if let Maybe::Some(t) = ty {
                    format!(": {}", self.format_type(t))
                } else {
                    String::new()
                };
                let val_str = if let Maybe::Some(v) = value {
                    format!(" = {}", self.format_expr(v))
                } else {
                    String::new()
                };
                format!("let {}{}{}", pat_str, ty_str, val_str)
            }
            verum_ast::stmt::StmtKind::Expr { expr, has_semi } => {
                if *has_semi {
                    format!("{};", self.format_expr(expr))
                } else {
                    self.format_expr(expr)
                }
            }
            verum_ast::stmt::StmtKind::LetElse {
                pattern,
                ty,
                value,
                else_block,
            } => {
                let pat_str = self.format_pattern(pattern);
                let ty_str = if let Maybe::Some(t) = ty {
                    format!(": {}", self.format_type(t))
                } else {
                    String::new()
                };
                format!(
                    "let {}{} = {} else {}",
                    pat_str,
                    ty_str,
                    self.format_expr(value),
                    self.format_block(else_block)
                )
            }
            verum_ast::stmt::StmtKind::Item(_) => "/* item */".to_string(),
            verum_ast::stmt::StmtKind::Defer(expr) => format!("defer {}", self.format_expr(expr)),
            verum_ast::stmt::StmtKind::Errdefer(expr) => {
                format!("errdefer {}", self.format_expr(expr))
            }
            verum_ast::stmt::StmtKind::Provide { context, value, alias } => {
                let alias_str = match alias {
                    Some(a) => format!(" as {}", a),
                    None => String::new(),
                };
                format!("provide {}{} = {}", context, alias_str, self.format_expr(value))
            }
            verum_ast::stmt::StmtKind::ProvideScope {
                context,
                value,
                block,
                alias,
            } => {
                let alias_str = match alias {
                    Some(a) => format!(" as {}", a),
                    None => String::new(),
                };
                format!(
                    "provide {}{} = {} in {}",
                    context,
                    alias_str,
                    self.format_expr(value),
                    self.format_expr(block)
                )
            }
            verum_ast::stmt::StmtKind::Empty => ";".to_string(),
        }
    }

    /// Format block
    fn format_block(&self, block: &verum_ast::Block) -> String {
        let mut lines = Vec::new();
        for stmt in &block.stmts {
            lines.push(self.format_stmt(stmt));
        }
        if let Maybe::Some(ref final_expr) = block.expr {
            lines.push(self.format_expr(final_expr));
        }
        format!("{{ {} }}", lines.join("; "))
    }

    /// Format if condition
    fn format_if_condition(&self, cond: &verum_ast::expr::IfCondition) -> String {
        let parts: Vec<String> = cond
            .conditions
            .iter()
            .map(|c| match c {
                verum_ast::expr::ConditionKind::Expr(e) => self.format_expr(e),
                verum_ast::expr::ConditionKind::Let { pattern, value } => {
                    format!(
                        "let {} = {}",
                        self.format_pattern(pattern),
                        self.format_expr(value)
                    )
                }
            })
            .collect();
        parts.join(" && ")
    }

    /// Format comprehension clause
    fn format_comprehension_clause(&self, clause: &verum_ast::ComprehensionClause) -> String {
        match &clause.kind {
            verum_ast::ComprehensionClauseKind::For { pattern, iter } => {
                format!(
                    "for {} in {}",
                    self.format_pattern(pattern),
                    self.format_expr(iter)
                )
            }
            verum_ast::ComprehensionClauseKind::If(cond) => {
                format!("if {}", self.format_expr(cond))
            }
            verum_ast::ComprehensionClauseKind::Let { pattern, ty, value } => {
                let ty_str = if let Maybe::Some(t) = ty {
                    format!(": {}", self.format_type(t))
                } else {
                    String::new()
                };
                format!(
                    "let {}{} = {}",
                    self.format_pattern(pattern),
                    ty_str,
                    self.format_expr(value)
                )
            }
        }
    }

    /// Format expression for OCaml
    fn format_expr_ocaml(&self, expr: &Expr) -> String {
        // Simplified OCaml formatting
        self.format_expr(expr)
    }
}

// ==================== Statistics ====================

/// Extraction statistics
#[derive(Debug, Clone, Default)]
pub struct ExtractionStats {
    /// Number of extraction attempts
    pub attempts: usize,

    /// Number of successful extractions
    pub successful: usize,

    /// Number of non-extractable proofs
    pub non_extractable: usize,

    /// Number of witness extractions
    pub witness_extractions: usize,
}

impl ExtractionStats {
    /// Get success rate
    pub fn success_rate(&self) -> f64 {
        if self.attempts == 0 {
            0.0
        } else {
            self.successful as f64 / self.attempts as f64
        }
    }
}

/// Erasure statistics
#[derive(Debug, Clone, Default)]
pub struct ErasureStats {
    /// Number of programs processed
    pub programs_processed: usize,

    /// Number of proofs erased
    pub proofs_erased: usize,
}

// ==================== Helper Types ====================

/// Proof arm for case analysis
#[derive(Debug, Clone)]
pub struct ProofArm {
    /// Pattern for this arm
    pub pattern: Text,

    /// Proof body
    pub body: ProofTerm,
}

impl ProofArm {
    /// Create a new proof arm
    pub fn new(pattern: Text, body: ProofTerm) -> Self {
        Self { pattern, body }
    }
}
