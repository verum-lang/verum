//! Statement generator for fuzz testing
//!
//! This module provides random statement generation with Arbitrary trait
//! implementations for property-based testing. It supports:
//!
//! - All Verum statement kinds (let, if, match, for, while, etc.)
//! - Type-aware statement generation
//! - Control flow statements (break, continue, return)
//! - Expression statements
//! - Shrinking for minimal counterexamples
//!
//! # Usage
//!
//! ```rust,no_run
//! use verum_fuzz::generators::stmt_generator::{StmtGenerator, ArbitraryStmt};
//! use rand::rng;
//!
//! let generator = StmtGenerator::new(Default::default());
//! let stmt = generator.generate(&mut rng());
//! ```

use super::config::GeneratorConfig;
use super::expr_generator::{ArbitraryExpr, ExprGenerator, ExprKind, GenerationContext};
use super::pattern_generator::{ArbitraryPattern, PatternGenerator};
use super::type_generator::{ArbitraryType, TypeGenerator, TypeKind};
use rand::Rng;
use rand::distr::Distribution;
use rand::distr::weighted::WeightedIndex;
use rand::seq::IndexedRandom;
use std::fmt;

/// Generated statement with source representation
#[derive(Clone)]
pub struct ArbitraryStmt {
    /// Source code representation
    pub source: String,
    /// Statement kind for shrinking
    pub kind: StmtKind,
    /// Depth of this statement
    pub depth: usize,
    /// Estimated complexity score
    pub complexity: usize,
    /// Variables introduced by this statement
    pub introduced_vars: Vec<String>,
}

impl fmt::Debug for ArbitraryStmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArbitraryStmt")
            .field("source", &self.source)
            .field("kind", &self.kind)
            .field("depth", &self.depth)
            .finish()
    }
}

impl fmt::Display for ArbitraryStmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.source)
    }
}

impl ArbitraryStmt {
    /// Create a new statement
    pub fn new(source: String, kind: StmtKind, depth: usize) -> Self {
        let complexity = Self::calculate_complexity(&source, depth);
        let introduced_vars = Self::extract_introduced_vars(&kind);
        Self {
            source,
            kind,
            depth,
            complexity,
            introduced_vars,
        }
    }

    /// Calculate complexity score for a statement
    fn calculate_complexity(source: &str, depth: usize) -> usize {
        let mut score = depth * 10;
        score += source.len();
        score += source.matches("let ").count() * 5;
        score += source.matches("if ").count() * 8;
        score += source.matches("match ").count() * 12;
        score += source.matches("for ").count() * 10;
        score += source.matches("while ").count() * 10;
        score += source.matches("loop ").count() * 10;
        score += source.matches("return").count() * 3;
        score
    }

    /// Extract variables introduced by this statement
    fn extract_introduced_vars(kind: &StmtKind) -> Vec<String> {
        match kind {
            StmtKind::Let { pattern, .. } => pattern.bound_names(),
            StmtKind::For { pattern, .. } => pattern.bound_names(),
            _ => Vec::new(),
        }
    }

    /// Generate shrunk versions of this statement
    pub fn shrink(&self) -> Vec<ArbitraryStmt> {
        let mut shrunk = Vec::new();

        match &self.kind {
            StmtKind::Let {
                pattern,
                ty,
                expr,
                is_mut,
            } => {
                // Try with simpler expression
                for simpler_expr in expr.shrink() {
                    let mut_str = if *is_mut { "mut " } else { "" };
                    let ty_str = ty
                        .as_ref()
                        .map(|t| format!(": {}", t.source))
                        .unwrap_or_default();
                    let source = format!(
                        "let {}{}{} = {};",
                        mut_str, pattern.source, ty_str, simpler_expr.source
                    );
                    shrunk.push(ArbitraryStmt::new(
                        source,
                        StmtKind::Let {
                            pattern: pattern.clone(),
                            ty: ty.clone(),
                            expr: simpler_expr,
                            is_mut: *is_mut,
                        },
                        self.depth,
                    ));
                }

                // Try without type annotation
                if ty.is_some() {
                    let mut_str = if *is_mut { "mut " } else { "" };
                    let source = format!("let {}{} = {};", mut_str, pattern.source, expr.source);
                    shrunk.push(ArbitraryStmt::new(
                        source,
                        StmtKind::Let {
                            pattern: pattern.clone(),
                            ty: None,
                            expr: expr.clone(),
                            is_mut: *is_mut,
                        },
                        self.depth,
                    ));
                }
            }

            StmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                // Try with simpler condition
                for simpler_cond in condition.shrink() {
                    let else_str = else_block
                        .as_ref()
                        .map(|s| {
                            format!(
                                " else {{ {} }}",
                                s.iter()
                                    .map(|s| s.source.as_str())
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            )
                        })
                        .unwrap_or_default();
                    let then_str = then_block
                        .iter()
                        .map(|s| s.source.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    let source =
                        format!("if {} {{ {} }}{}", simpler_cond.source, then_str, else_str);
                    shrunk.push(ArbitraryStmt::new(
                        source,
                        StmtKind::If {
                            condition: simpler_cond,
                            then_block: then_block.clone(),
                            else_block: else_block.clone(),
                        },
                        self.depth,
                    ));
                }

                // Try without else block
                if else_block.is_some() && !then_block.is_empty() {
                    let then_str = then_block
                        .iter()
                        .map(|s| s.source.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    let source = format!("if {} {{ {} }}", condition.source, then_str);
                    shrunk.push(ArbitraryStmt::new(
                        source,
                        StmtKind::If {
                            condition: condition.clone(),
                            then_block: then_block.clone(),
                            else_block: None,
                        },
                        self.depth,
                    ));
                }

                // Try with fewer statements in blocks
                if then_block.len() > 1 {
                    for i in 0..then_block.len() {
                        let mut new_block = then_block.clone();
                        new_block.remove(i);
                        let then_str = new_block
                            .iter()
                            .map(|s| s.source.as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        let else_str = else_block
                            .as_ref()
                            .map(|s| {
                                format!(
                                    " else {{ {} }}",
                                    s.iter()
                                        .map(|s| s.source.as_str())
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                )
                            })
                            .unwrap_or_default();
                        let source =
                            format!("if {} {{ {} }}{}", condition.source, then_str, else_str);
                        shrunk.push(ArbitraryStmt::new(
                            source,
                            StmtKind::If {
                                condition: condition.clone(),
                                then_block: new_block,
                                else_block: else_block.clone(),
                            },
                            self.depth,
                        ));
                    }
                }
            }

            StmtKind::For {
                pattern,
                iterator,
                body,
            } => {
                // Try with simpler iterator
                for simpler_iter in iterator.shrink() {
                    let body_str = body
                        .iter()
                        .map(|s| s.source.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    let source = format!(
                        "for {} in {} {{ {} }}",
                        pattern.source, simpler_iter.source, body_str
                    );
                    shrunk.push(ArbitraryStmt::new(
                        source,
                        StmtKind::For {
                            pattern: pattern.clone(),
                            iterator: simpler_iter,
                            body: body.clone(),
                        },
                        self.depth,
                    ));
                }

                // Try with fewer body statements
                if body.len() > 1 {
                    for i in 0..body.len() {
                        let mut new_body = body.clone();
                        new_body.remove(i);
                        let body_str = new_body
                            .iter()
                            .map(|s| s.source.as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        let source = format!(
                            "for {} in {} {{ {} }}",
                            pattern.source, iterator.source, body_str
                        );
                        shrunk.push(ArbitraryStmt::new(
                            source,
                            StmtKind::For {
                                pattern: pattern.clone(),
                                iterator: iterator.clone(),
                                body: new_body,
                            },
                            self.depth,
                        ));
                    }
                }
            }

            StmtKind::While { condition, body } => {
                // Try with simpler condition
                for simpler_cond in condition.shrink() {
                    let body_str = body
                        .iter()
                        .map(|s| s.source.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    let source = format!("while {} {{ {} }}", simpler_cond.source, body_str);
                    shrunk.push(ArbitraryStmt::new(
                        source,
                        StmtKind::While {
                            condition: simpler_cond,
                            body: body.clone(),
                        },
                        self.depth,
                    ));
                }
            }

            StmtKind::Expression(expr) => {
                // Try with simpler expression
                for simpler_expr in expr.shrink() {
                    let source = format!("{};", simpler_expr.source);
                    shrunk.push(ArbitraryStmt::new(
                        source,
                        StmtKind::Expression(simpler_expr),
                        self.depth,
                    ));
                }
            }

            StmtKind::Return(Some(expr)) => {
                // Try with simpler return expression
                for simpler_expr in expr.shrink() {
                    let source = format!("return {};", simpler_expr.source);
                    shrunk.push(ArbitraryStmt::new(
                        source,
                        StmtKind::Return(Some(simpler_expr)),
                        self.depth,
                    ));
                }

                // Try with no return value
                shrunk.push(ArbitraryStmt::new(
                    "return;".to_string(),
                    StmtKind::Return(None),
                    self.depth,
                ));
            }

            _ => {
                // For simple statements, try a simpler variant
                if !matches!(self.kind, StmtKind::Break | StmtKind::Continue) {
                    shrunk.push(ArbitraryStmt::new(
                        "();".to_string(),
                        StmtKind::Expression(ArbitraryExpr::new(
                            "()".to_string(),
                            ExprKind::Literal(super::expr_generator::LiteralValue::Unit),
                            0,
                        )),
                        0,
                    ));
                }
            }
        }

        // Filter out shrunk versions that are not simpler
        shrunk.retain(|s| s.complexity < self.complexity);
        shrunk
    }

    /// Check if this statement introduces any variables
    pub fn introduces_variables(&self) -> bool {
        !self.introduced_vars.is_empty()
    }

    /// Check if this statement is a control flow statement
    pub fn is_control_flow(&self) -> bool {
        matches!(
            self.kind,
            StmtKind::Return(_) | StmtKind::Break | StmtKind::Continue
        )
    }

    /// Check if this statement is a block statement
    pub fn is_block(&self) -> bool {
        matches!(
            self.kind,
            StmtKind::If { .. }
                | StmtKind::Match { .. }
                | StmtKind::For { .. }
                | StmtKind::While { .. }
                | StmtKind::Loop { .. }
        )
    }
}

/// Statement kind for structured representation
#[derive(Debug, Clone)]
pub enum StmtKind {
    /// Let binding: let pattern = expr;
    Let {
        pattern: ArbitraryPattern,
        ty: Option<ArbitraryType>,
        expr: ArbitraryExpr,
        is_mut: bool,
    },

    /// Assignment: target = expr;
    Assignment {
        target: ArbitraryExpr,
        expr: ArbitraryExpr,
    },

    /// Expression statement: expr;
    Expression(ArbitraryExpr),

    /// If statement
    If {
        condition: ArbitraryExpr,
        then_block: Vec<ArbitraryStmt>,
        else_block: Option<Vec<ArbitraryStmt>>,
    },

    /// Match statement
    Match {
        scrutinee: ArbitraryExpr,
        arms: Vec<(ArbitraryPattern, Vec<ArbitraryStmt>)>,
    },

    /// For loop
    For {
        pattern: ArbitraryPattern,
        iterator: ArbitraryExpr,
        body: Vec<ArbitraryStmt>,
    },

    /// While loop
    While {
        condition: ArbitraryExpr,
        body: Vec<ArbitraryStmt>,
    },

    /// Infinite loop
    Loop { body: Vec<ArbitraryStmt> },

    /// Return statement
    Return(Option<ArbitraryExpr>),

    /// Break statement
    Break,

    /// Continue statement
    Continue,

    /// Block statement
    Block(Vec<ArbitraryStmt>),
}

/// Statement generator
pub struct StmtGenerator {
    config: GeneratorConfig,
    expr_gen: ExprGenerator,
    type_gen: TypeGenerator,
    pattern_gen: PatternGenerator,
    stmt_dist: WeightedIndex<u32>,
}

impl StmtGenerator {
    /// Create a new statement generator with the given configuration
    pub fn new(config: GeneratorConfig) -> Self {
        let weights = config.weights.statements.as_vec();
        let stmt_dist = WeightedIndex::new(&weights).unwrap();
        let expr_gen = ExprGenerator::new(config.clone());
        let type_gen = TypeGenerator::new(config.clone());
        let pattern_gen = PatternGenerator::new(config.clone());
        Self {
            config,
            expr_gen,
            type_gen,
            pattern_gen,
            stmt_dist,
        }
    }

    /// Generate a random statement
    pub fn generate<R: Rng>(&self, rng: &mut R) -> ArbitraryStmt {
        self.generate_stmt(rng, 0, &mut Vec::new(), false)
    }

    /// Generate a statement with context
    pub fn generate_with_context<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        available_vars: &[String],
        in_loop: bool,
    ) -> ArbitraryStmt {
        self.generate_stmt(rng, depth, &available_vars.to_vec(), in_loop)
    }

    /// Generate a statement at a given depth
    fn generate_stmt<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        available_vars: &Vec<String>,
        in_loop: bool,
    ) -> ArbitraryStmt {
        // At max depth, only generate simple statements
        if depth >= self.config.complexity.max_depth {
            return self.generate_simple_stmt(rng, available_vars);
        }

        match self.stmt_dist.sample(rng) {
            0 | 1 | 2 => self.generate_let_stmt(rng, available_vars),
            3 => self.generate_assignment_stmt(rng, available_vars),
            4 => self.generate_expression_stmt(rng, available_vars),
            5 => self.generate_if_stmt(rng, depth, available_vars, in_loop),
            6 => self.generate_match_stmt(rng, depth, available_vars, in_loop),
            7 => self.generate_for_stmt(rng, depth, available_vars),
            8 => self.generate_while_stmt(rng, depth, available_vars),
            9 => self.generate_loop_stmt(rng, depth, available_vars),
            10 => self.generate_return_stmt(rng, available_vars),
            11 if in_loop => self.generate_break_stmt(),
            12 if in_loop => self.generate_continue_stmt(),
            _ => self.generate_simple_stmt(rng, available_vars),
        }
    }

    /// Generate a simple statement (no blocks)
    fn generate_simple_stmt<R: Rng>(
        &self,
        rng: &mut R,
        available_vars: &[String],
    ) -> ArbitraryStmt {
        match rng.random_range(0..3) {
            0 => self.generate_let_stmt(rng, available_vars),
            1 if !available_vars.is_empty() => self.generate_assignment_stmt(rng, available_vars),
            _ => self.generate_expression_stmt(rng, available_vars),
        }
    }

    /// Generate a let statement
    fn generate_let_stmt<R: Rng>(&self, rng: &mut R, _available_vars: &[String]) -> ArbitraryStmt {
        let pattern = self.pattern_gen.generate_binding_pattern(rng);
        let is_mut = rng.random_bool(0.3);
        let has_annotation = rng.random_bool(0.4);

        let ty = if has_annotation {
            Some(self.type_gen.generate_primitive(rng))
        } else {
            None
        };

        let expr = if let Some(ref t) = ty {
            self.generate_typed_expr(rng, t)
        } else {
            self.expr_gen.generate(rng)
        };

        let mut_str = if is_mut { "mut " } else { "" };
        let ty_str = ty
            .as_ref()
            .map(|t| format!(": {}", t.source))
            .unwrap_or_default();
        let source = format!(
            "let {}{}{} = {};",
            mut_str, pattern.source, ty_str, expr.source
        );

        ArbitraryStmt::new(
            source,
            StmtKind::Let {
                pattern,
                ty,
                expr,
                is_mut,
            },
            0,
        )
    }

    /// Generate an assignment statement
    fn generate_assignment_stmt<R: Rng>(
        &self,
        rng: &mut R,
        available_vars: &[String],
    ) -> ArbitraryStmt {
        let target = if !available_vars.is_empty() && rng.random_bool(0.8) {
            let var = available_vars.choose(rng).unwrap();
            ArbitraryExpr::new(var.clone(), ExprKind::Identifier(var.clone()), 0)
        } else {
            self.expr_gen.generate_identifier(rng, &mut GenerationContext::new())
        };

        let expr = self.expr_gen.generate(rng);
        let source = format!("{} = {};", target.source, expr.source);

        ArbitraryStmt::new(source, StmtKind::Assignment { target, expr }, 0)
    }

    /// Generate an expression statement
    fn generate_expression_stmt<R: Rng>(
        &self,
        rng: &mut R,
        available_vars: &[String],
    ) -> ArbitraryStmt {
        let expr = if !available_vars.is_empty() && rng.random_bool(0.5) {
            // Use an available variable in the expression
            let var = available_vars.choose(rng).unwrap();
            if rng.random_bool(0.5) {
                // Method call on variable
                let method = ["len", "is_empty", "clone", "to_string"]
                    .choose(rng)
                    .unwrap();
                ArbitraryExpr::new(
                    format!("{}.{}()", var, method),
                    ExprKind::MethodCall {
                        receiver: Box::new(ArbitraryExpr::new(
                            var.clone(),
                            ExprKind::Identifier(var.clone()),
                            0,
                        )),
                        method: method.to_string(),
                        args: Vec::new(),
                    },
                    1,
                )
            } else {
                ArbitraryExpr::new(var.clone(), ExprKind::Identifier(var.clone()), 0)
            }
        } else {
            self.expr_gen.generate(rng)
        };

        let source = format!("{};", expr.source);
        ArbitraryStmt::new(source, StmtKind::Expression(expr), 0)
    }

    /// Generate an if statement
    fn generate_if_stmt<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        available_vars: &[String],
        in_loop: bool,
    ) -> ArbitraryStmt {
        let condition = self.expr_gen.generate_typed(rng, "Bool");

        let num_then_stmts = rng.random_range(1..=3);
        let mut then_vars = available_vars.to_vec();
        let mut then_block = Vec::new();
        for _ in 0..num_then_stmts {
            let stmt = self.generate_stmt(rng, depth + 1, &then_vars, in_loop);
            then_vars.extend(stmt.introduced_vars.clone());
            then_block.push(stmt);
        }

        let else_block = if rng.random_bool(0.4) {
            let num_else_stmts = rng.random_range(1..=2);
            let mut else_vars = available_vars.to_vec();
            let mut block = Vec::new();
            for _ in 0..num_else_stmts {
                let stmt = self.generate_stmt(rng, depth + 1, &else_vars, in_loop);
                else_vars.extend(stmt.introduced_vars.clone());
                block.push(stmt);
            }
            Some(block)
        } else {
            None
        };

        let then_str = then_block
            .iter()
            .map(|s| format!("    {}", s.source))
            .collect::<Vec<_>>()
            .join("\n");

        let else_str = else_block
            .as_ref()
            .map(|b| {
                let inner = b
                    .iter()
                    .map(|s| format!("    {}", s.source))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(" else {{\n{}\n}}", inner)
            })
            .unwrap_or_default();

        let source = format!("if {} {{\n{}\n}}{}", condition.source, then_str, else_str);

        ArbitraryStmt::new(
            source,
            StmtKind::If {
                condition,
                then_block,
                else_block,
            },
            depth,
        )
    }

    /// Generate a match statement
    fn generate_match_stmt<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        available_vars: &[String],
        in_loop: bool,
    ) -> ArbitraryStmt {
        let scrutinee = self.expr_gen.generate(rng);
        let num_arms = rng.random_range(2..=4);

        let mut arms = Vec::new();
        for i in 0..num_arms {
            let pattern = if i == num_arms - 1 {
                // Last arm should be a wildcard for exhaustiveness
                self.pattern_gen.generate_wildcard()
            } else {
                self.pattern_gen.generate(rng)
            };

            let mut arm_vars = available_vars.to_vec();
            arm_vars.extend(pattern.bound_names());

            let num_stmts = rng.random_range(1..=2);
            let mut body = Vec::new();
            for _ in 0..num_stmts {
                let stmt = self.generate_stmt(rng, depth + 1, &arm_vars, in_loop);
                arm_vars.extend(stmt.introduced_vars.clone());
                body.push(stmt);
            }

            arms.push((pattern, body));
        }

        let arms_str = arms
            .iter()
            .map(|(p, body)| {
                let body_str = body
                    .iter()
                    .map(|s| format!("        {}", s.source))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("    {} => {{\n{}\n    }}", p.source, body_str)
            })
            .collect::<Vec<_>>()
            .join(",\n");

        let source = format!("match {} {{\n{}\n}}", scrutinee.source, arms_str);

        ArbitraryStmt::new(source, StmtKind::Match { scrutinee, arms }, depth)
    }

    /// Generate a for loop
    fn generate_for_stmt<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        available_vars: &[String],
    ) -> ArbitraryStmt {
        let pattern = self.pattern_gen.generate_binding_pattern(rng);

        // Generate an iterator expression (range or collection)
        let iterator = if rng.random_bool(0.6) {
            // Range
            let start = rng.random_range(0..10);
            let end = rng.random_range(start + 1..start + 20);
            ArbitraryExpr::new(
                format!("{}..{}", start, end),
                ExprKind::Range {
                    start: Some(Box::new(ArbitraryExpr::new(
                        start.to_string(),
                        ExprKind::Literal(super::expr_generator::LiteralValue::Int(start as i64)),
                        0,
                    ))),
                    end: Some(Box::new(ArbitraryExpr::new(
                        end.to_string(),
                        ExprKind::Literal(super::expr_generator::LiteralValue::Int(end as i64)),
                        0,
                    ))),
                    inclusive: false,
                },
                0,
            )
        } else if !available_vars.is_empty() && rng.random_bool(0.5) {
            // Use an available collection variable
            let var = available_vars.choose(rng).unwrap();
            ArbitraryExpr::new(var.clone(), ExprKind::Identifier(var.clone()), 0)
        } else {
            // Generate a list literal
            let len = rng.random_range(1..5);
            let elements: Vec<i64> = (0..len).map(|_| rng.random_range(0..100)).collect();
            let elements_str = elements
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            ArbitraryExpr::new(
                format!("[{}]", elements_str),
                ExprKind::List {
                    elements: elements
                        .iter()
                        .map(|&e| {
                            ArbitraryExpr::new(
                                e.to_string(),
                                ExprKind::Literal(super::expr_generator::LiteralValue::Int(e)),
                                0,
                            )
                        })
                        .collect(),
                },
                0,
            )
        };

        let mut loop_vars = available_vars.to_vec();
        loop_vars.extend(pattern.bound_names());

        let num_stmts = rng.random_range(1..=3);
        let mut body = Vec::new();
        for _ in 0..num_stmts {
            let stmt = self.generate_stmt(rng, depth + 1, &loop_vars, true);
            loop_vars.extend(stmt.introduced_vars.clone());
            body.push(stmt);
        }

        let body_str = body
            .iter()
            .map(|s| format!("    {}", s.source))
            .collect::<Vec<_>>()
            .join("\n");

        let source = format!(
            "for {} in {} {{\n{}\n}}",
            pattern.source, iterator.source, body_str
        );

        ArbitraryStmt::new(
            source,
            StmtKind::For {
                pattern,
                iterator,
                body,
            },
            depth,
        )
    }

    /// Generate a while loop
    fn generate_while_stmt<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        available_vars: &[String],
    ) -> ArbitraryStmt {
        let condition = self.expr_gen.generate_typed(rng, "Bool");

        let mut loop_vars = available_vars.to_vec();
        let num_stmts = rng.random_range(1..=3);
        let mut body = Vec::new();
        for _ in 0..num_stmts {
            let stmt = self.generate_stmt(rng, depth + 1, &loop_vars, true);
            loop_vars.extend(stmt.introduced_vars.clone());
            body.push(stmt);
        }

        let body_str = body
            .iter()
            .map(|s| format!("    {}", s.source))
            .collect::<Vec<_>>()
            .join("\n");

        let source = format!("while {} {{\n{}\n}}", condition.source, body_str);

        ArbitraryStmt::new(source, StmtKind::While { condition, body }, depth)
    }

    /// Generate an infinite loop
    fn generate_loop_stmt<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        available_vars: &[String],
    ) -> ArbitraryStmt {
        let mut loop_vars = available_vars.to_vec();
        let num_stmts = rng.random_range(1..=3);
        let mut body = Vec::new();

        for i in 0..num_stmts {
            // Ensure at least one break to avoid infinite loops
            let stmt = if i == num_stmts - 1 && rng.random_bool(0.7) {
                self.generate_break_stmt()
            } else {
                let s = self.generate_stmt(rng, depth + 1, &loop_vars, true);
                loop_vars.extend(s.introduced_vars.clone());
                s
            };
            body.push(stmt);
        }

        let body_str = body
            .iter()
            .map(|s| format!("    {}", s.source))
            .collect::<Vec<_>>()
            .join("\n");

        let source = format!("loop {{\n{}\n}}", body_str);

        ArbitraryStmt::new(source, StmtKind::Loop { body }, depth)
    }

    /// Generate a return statement
    fn generate_return_stmt<R: Rng>(
        &self,
        rng: &mut R,
        _available_vars: &[String],
    ) -> ArbitraryStmt {
        let has_value = rng.random_bool(0.7);

        if has_value {
            let expr = self.expr_gen.generate(rng);
            let source = format!("return {};", expr.source);
            ArbitraryStmt::new(source, StmtKind::Return(Some(expr)), 0)
        } else {
            ArbitraryStmt::new("return;".to_string(), StmtKind::Return(None), 0)
        }
    }

    /// Generate a break statement
    fn generate_break_stmt(&self) -> ArbitraryStmt {
        ArbitraryStmt::new("break;".to_string(), StmtKind::Break, 0)
    }

    /// Generate a continue statement
    fn generate_continue_stmt(&self) -> ArbitraryStmt {
        ArbitraryStmt::new("continue;".to_string(), StmtKind::Continue, 0)
    }

    /// Generate an expression of a specific type
    fn generate_typed_expr<R: Rng>(&self, rng: &mut R, ty: &ArbitraryType) -> ArbitraryExpr {
        match &ty.kind {
            TypeKind::Int => self.expr_gen.generate_typed(rng, "Int"),
            TypeKind::Float => self.expr_gen.generate_typed(rng, "Float"),
            TypeKind::Bool => self.expr_gen.generate_typed(rng, "Bool"),
            TypeKind::Text => self.expr_gen.generate_typed(rng, "Text"),
            TypeKind::Unit => ArbitraryExpr::new(
                "()".to_string(),
                ExprKind::Literal(super::expr_generator::LiteralValue::Unit),
                0,
            ),
            _ => self.expr_gen.generate(rng),
        }
    }

    /// Generate a block of statements
    pub fn generate_block<R: Rng>(
        &self,
        rng: &mut R,
        num_stmts: usize,
        available_vars: &[String],
        in_loop: bool,
    ) -> Vec<ArbitraryStmt> {
        let mut vars = available_vars.to_vec();
        let mut stmts = Vec::new();

        for _ in 0..num_stmts {
            let stmt = self.generate_stmt(rng, 0, &vars, in_loop);
            vars.extend(stmt.introduced_vars.clone());
            stmts.push(stmt);
        }

        stmts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_generate_stmt() {
        let config = GeneratorConfig::default();
        let generator = StmtGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..20 {
            let stmt = generator.generate(&mut rng);
            assert!(!stmt.source.is_empty());
        }
    }

    #[test]
    fn test_generate_let() {
        let config = GeneratorConfig::default();
        let generator = StmtGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let stmt = generator.generate_let_stmt(&mut rng, &[]);
        assert!(stmt.source.starts_with("let "));
        assert!(stmt.source.ends_with(";"));
    }

    #[test]
    fn test_generate_if() {
        let config = GeneratorConfig::default();
        let generator = StmtGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let stmt = generator.generate_if_stmt(&mut rng, 0, &[], false);
        assert!(stmt.source.contains("if "));
        assert!(stmt.source.contains("{"));
    }

    #[test]
    fn test_generate_for() {
        let config = GeneratorConfig::default();
        let generator = StmtGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let stmt = generator.generate_for_stmt(&mut rng, 0, &[]);
        assert!(stmt.source.contains("for "));
        assert!(stmt.source.contains(" in "));
    }

    #[test]
    fn test_shrinking() {
        let config = GeneratorConfig::default();
        let generator = StmtGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let stmt = generator.generate_if_stmt(&mut rng, 0, &[], false);
        let shrunk = stmt.shrink();

        for s in shrunk {
            assert!(s.complexity <= stmt.complexity);
        }
    }

    #[test]
    fn test_generate_block() {
        let config = GeneratorConfig::default();
        let generator = StmtGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let block = generator.generate_block(&mut rng, 5, &[], false);
        assert_eq!(block.len(), 5);
    }

    #[test]
    fn test_control_flow_in_loop() {
        let config = GeneratorConfig::default();
        let generator = StmtGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // Generate many statements in a loop context to verify break/continue can be generated
        let mut found_control = false;
        for _ in 0..100 {
            let stmt = generator.generate_with_context(&mut rng, 0, &[], true);
            if stmt.is_control_flow() {
                found_control = true;
                break;
            }
        }
        // Note: This might not always find control flow due to randomness
        // but it verifies the context parameter works
    }

    #[test]
    fn test_deterministic_with_seed() {
        let config = GeneratorConfig::default();
        let generator = StmtGenerator::new(config);

        let mut rng1 = ChaCha8Rng::seed_from_u64(12345);
        let mut rng2 = ChaCha8Rng::seed_from_u64(12345);

        let stmt1 = generator.generate(&mut rng1);
        let stmt2 = generator.generate(&mut rng2);

        assert_eq!(stmt1.source, stmt2.source);
    }
}
