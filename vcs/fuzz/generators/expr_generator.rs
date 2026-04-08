//! Expression generator for fuzz testing
//!
//! This module provides random expression generation with Arbitrary trait
//! implementations for property-based testing. It supports:
//!
//! - All Verum expression types (literals, binary, unary, etc.)
//! - Configurable depth and complexity limits
//! - Shrinking for minimal counterexamples
//! - Type-aware generation for valid expressions
//!
//! # Arbitrary Trait
//!
//! The Arbitrary trait implementation allows expressions to be used with
//! property-based testing frameworks like proptest and quickcheck.
//!
//! # Usage
//!
//! ```rust,no_run
//! use verum_fuzz::generators::expr_generator::{ExprGenerator, ArbitraryExpr};
//! use rand::rng;
//!
//! let generator = ExprGenerator::new(Default::default());
//! let expr = generator.generate(&mut rng());
//! ```

use super::config::GeneratorConfig;
use rand::Rng;
use rand::distr::Distribution;
use rand::distr::weighted::WeightedIndex;
use rand::seq::IndexedRandom;
use std::fmt;

/// Generated expression with source representation
#[derive(Clone, PartialEq, Eq)]
pub struct ArbitraryExpr {
    /// Source code representation
    pub source: String,
    /// Expression kind for shrinking
    pub kind: ExprKind,
    /// Depth of this expression
    pub depth: usize,
    /// Estimated complexity score
    pub complexity: usize,
}

impl fmt::Debug for ArbitraryExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArbitraryExpr")
            .field("source", &self.source)
            .field("kind", &self.kind)
            .field("depth", &self.depth)
            .finish()
    }
}

impl fmt::Display for ArbitraryExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.source)
    }
}

impl ArbitraryExpr {
    /// Create a new expression
    pub fn new(source: String, kind: ExprKind, depth: usize) -> Self {
        let complexity = Self::calculate_complexity(&source, depth);
        Self {
            source,
            kind,
            depth,
            complexity,
        }
    }

    /// Calculate complexity score for an expression
    fn calculate_complexity(source: &str, depth: usize) -> usize {
        let mut score = depth * 10;
        score += source.len();
        score += source.matches('(').count() * 2;
        score += source.matches('{').count() * 3;
        score += source.matches("if ").count() * 5;
        score += source.matches("match ").count() * 8;
        score += source.matches("async ").count() * 4;
        score
    }

    /// Generate shrunk versions of this expression
    pub fn shrink(&self) -> Vec<ArbitraryExpr> {
        let mut shrunk = Vec::new();

        match &self.kind {
            ExprKind::Binary { left, right, .. } => {
                // Try just left or right operand
                shrunk.push(left.as_ref().clone());
                shrunk.push(right.as_ref().clone());
            }
            ExprKind::Unary { expr, .. } => {
                // Try without the unary operator
                shrunk.push(expr.as_ref().clone());
            }
            ExprKind::Call { args, .. } => {
                // Try with fewer arguments
                for arg in args {
                    shrunk.push(arg.clone());
                }
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Try just the branches
                shrunk.push(condition.as_ref().clone());
                shrunk.push(then_branch.as_ref().clone());
                if let Some(else_branch) = else_branch {
                    shrunk.push(else_branch.as_ref().clone());
                }
            }
            ExprKind::Block { exprs, .. } => {
                // Try with fewer expressions
                for expr in exprs {
                    shrunk.push(expr.clone());
                }
            }
            ExprKind::List { elements } => {
                // Try with fewer elements
                for elem in elements {
                    shrunk.push(elem.clone());
                }
                // Try empty list
                shrunk.push(ArbitraryExpr::new(
                    "[]".to_string(),
                    ExprKind::List { elements: vec![] },
                    0,
                ));
            }
            ExprKind::Tuple { elements } => {
                // Try individual elements
                for elem in elements {
                    shrunk.push(elem.clone());
                }
            }
            ExprKind::Literal(lit) => {
                // Shrink literals
                shrunk.extend(
                    lit.shrink()
                        .into_iter()
                        .map(|l| ArbitraryExpr::new(l.to_string(), ExprKind::Literal(l), 0)),
                );
            }
            _ => {
                // For simple expressions, try simpler literals
                shrunk.push(ArbitraryExpr::new(
                    "0".to_string(),
                    ExprKind::Literal(LiteralValue::Int(0)),
                    0,
                ));
                shrunk.push(ArbitraryExpr::new(
                    "true".to_string(),
                    ExprKind::Literal(LiteralValue::Bool(true)),
                    0,
                ));
            }
        }

        // Filter out expressions that are not simpler
        shrunk.retain(|s| s.complexity < self.complexity);
        shrunk
    }
}

/// Expression kind for structured representation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprKind {
    /// Literal value
    Literal(LiteralValue),
    /// Identifier reference
    Identifier(String),
    /// Binary operation
    Binary {
        op: BinaryOp,
        left: Box<ArbitraryExpr>,
        right: Box<ArbitraryExpr>,
    },
    /// Unary operation
    Unary {
        op: UnaryOp,
        expr: Box<ArbitraryExpr>,
    },
    /// Function call
    Call {
        func: String,
        args: Vec<ArbitraryExpr>,
    },
    /// Method call
    MethodCall {
        receiver: Box<ArbitraryExpr>,
        method: String,
        args: Vec<ArbitraryExpr>,
    },
    /// Field access
    FieldAccess {
        expr: Box<ArbitraryExpr>,
        field: String,
    },
    /// Index access
    Index {
        expr: Box<ArbitraryExpr>,
        index: Box<ArbitraryExpr>,
    },
    /// If expression
    If {
        condition: Box<ArbitraryExpr>,
        then_branch: Box<ArbitraryExpr>,
        else_branch: Option<Box<ArbitraryExpr>>,
    },
    /// Match expression
    Match {
        scrutinee: Box<ArbitraryExpr>,
        arms: Vec<(String, ArbitraryExpr)>,
    },
    /// Block expression
    Block {
        exprs: Vec<ArbitraryExpr>,
        trailing: Option<Box<ArbitraryExpr>>,
    },
    /// Lambda/closure
    Lambda {
        params: Vec<String>,
        body: Box<ArbitraryExpr>,
    },
    /// Tuple expression
    Tuple { elements: Vec<ArbitraryExpr> },
    /// List expression
    List { elements: Vec<ArbitraryExpr> },
    /// Range expression
    Range {
        start: Option<Box<ArbitraryExpr>>,
        end: Option<Box<ArbitraryExpr>>,
        inclusive: bool,
    },
    /// Try expression
    Try(Box<ArbitraryExpr>),
    /// Await expression
    Await(Box<ArbitraryExpr>),
    /// Async block
    Async(Box<ArbitraryExpr>),
    /// Spawn expression
    Spawn(Box<ArbitraryExpr>),
    /// Reference
    Ref {
        mutable: bool,
        tier: RefTier,
        expr: Box<ArbitraryExpr>,
    },
    /// Parenthesized expression
    Paren(Box<ArbitraryExpr>),
}

/// Literal values
#[derive(Debug, Clone, PartialEq)]
pub enum LiteralValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
    Char(char),
    Unit,
}

impl Eq for LiteralValue {}

impl fmt::Display for LiteralValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LiteralValue::Int(n) => write!(f, "{}", n),
            LiteralValue::Float(n) => write!(f, "{:.2}", n),
            LiteralValue::Bool(b) => write!(f, "{}", b),
            LiteralValue::Text(s) => write!(f, "\"{}\"", s),
            LiteralValue::Char(c) => write!(f, "'{}'", c),
            LiteralValue::Unit => write!(f, "()"),
        }
    }
}

impl LiteralValue {
    /// Shrink a literal value
    pub fn shrink(&self) -> Vec<LiteralValue> {
        match self {
            LiteralValue::Int(n) => {
                let mut shrunk = vec![];
                if *n != 0 {
                    shrunk.push(LiteralValue::Int(0));
                    shrunk.push(LiteralValue::Int(n / 2));
                    if *n > 0 {
                        shrunk.push(LiteralValue::Int(n - 1));
                    } else {
                        shrunk.push(LiteralValue::Int(n + 1));
                    }
                }
                shrunk
            }
            LiteralValue::Float(n) => {
                vec![
                    LiteralValue::Float(0.0),
                    LiteralValue::Float(n / 2.0),
                    LiteralValue::Float(n.floor()),
                ]
            }
            LiteralValue::Text(s) => {
                let mut shrunk = vec![];
                if !s.is_empty() {
                    shrunk.push(LiteralValue::Text(String::new()));
                    shrunk.push(LiteralValue::Text(s[..s.len() / 2].to_string()));
                    shrunk.push(LiteralValue::Text(s[1..].to_string()));
                }
                shrunk
            }
            _ => vec![],
        }
    }
}

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Pipeline,
    NullCoalesce,
}

impl BinaryOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Rem => "%",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
            BinaryOp::BitAnd => "&",
            BinaryOp::BitOr => "|",
            BinaryOp::BitXor => "^",
            BinaryOp::Shl => "<<",
            BinaryOp::Shr => ">>",
            BinaryOp::Pipeline => "|>",
            BinaryOp::NullCoalesce => "??",
        }
    }

    pub fn all() -> &'static [BinaryOp] {
        &[
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Mul,
            BinaryOp::Div,
            BinaryOp::Rem,
            BinaryOp::Eq,
            BinaryOp::Ne,
            BinaryOp::Lt,
            BinaryOp::Le,
            BinaryOp::Gt,
            BinaryOp::Ge,
            BinaryOp::And,
            BinaryOp::Or,
            BinaryOp::BitAnd,
            BinaryOp::BitOr,
            BinaryOp::BitXor,
            BinaryOp::Shl,
            BinaryOp::Shr,
        ]
    }

    pub fn arithmetic() -> &'static [BinaryOp] {
        &[
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Mul,
            BinaryOp::Div,
            BinaryOp::Rem,
        ]
    }

    pub fn comparison() -> &'static [BinaryOp] {
        &[
            BinaryOp::Eq,
            BinaryOp::Ne,
            BinaryOp::Lt,
            BinaryOp::Le,
            BinaryOp::Gt,
            BinaryOp::Ge,
        ]
    }

    pub fn logical() -> &'static [BinaryOp] {
        &[BinaryOp::And, BinaryOp::Or]
    }
}

/// Unary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
    Deref,
}

impl UnaryOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            UnaryOp::Neg => "-",
            UnaryOp::Not => "!",
            UnaryOp::BitNot => "~",
            UnaryOp::Deref => "*",
        }
    }
}

/// CBGR reference tier
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefTier {
    Managed,
    Checked,
    Unsafe,
}

impl RefTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            RefTier::Managed => "&",
            RefTier::Checked => "&checked ",
            RefTier::Unsafe => "&unsafe ",
        }
    }
}

/// Expression generator
pub struct ExprGenerator {
    config: GeneratorConfig,
    expr_dist: WeightedIndex<u32>,
}

impl ExprGenerator {
    /// Create a new expression generator with the given configuration
    pub fn new(config: GeneratorConfig) -> Self {
        let weights = config.weights.expressions.as_vec();
        let expr_dist = WeightedIndex::new(&weights).unwrap();
        Self { config, expr_dist }
    }

    /// Generate a random expression
    pub fn generate<R: Rng>(&self, rng: &mut R) -> ArbitraryExpr {
        self.generate_expr(rng, 0, &mut GenerationContext::new())
    }

    /// Generate an expression with a specific target type
    pub fn generate_typed<R: Rng>(&self, rng: &mut R, target_type: &str) -> ArbitraryExpr {
        let mut ctx = GenerationContext::new();
        match target_type {
            "Int" => self.generate_int_expr(rng, 0, &mut ctx),
            "Float" => self.generate_float_expr(rng, 0, &mut ctx),
            "Bool" => self.generate_bool_expr(rng, 0, &mut ctx),
            "Text" => self.generate_text_expr(rng, 0, &mut ctx),
            _ => self.generate_expr(rng, 0, &mut ctx),
        }
    }

    /// Generate an expression at a given depth
    fn generate_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        // At max depth, generate only simple expressions
        if depth >= self.config.complexity.max_depth {
            return self.generate_simple_expr(rng, ctx);
        }

        match self.expr_dist.sample(rng) {
            0 => self.generate_literal(rng),
            1 => self.generate_identifier(rng, ctx),
            2 => self.generate_binary_expr(rng, depth, ctx),
            3 => self.generate_unary_expr(rng, depth, ctx),
            4 => self.generate_call_expr(rng, depth, ctx),
            5 => self.generate_method_call(rng, depth, ctx),
            6 => self.generate_field_access(rng, depth, ctx),
            7 => self.generate_index_expr(rng, depth, ctx),
            8 => self.generate_if_expr(rng, depth, ctx),
            9 => self.generate_match_expr(rng, depth, ctx),
            10 => self.generate_block_expr(rng, depth, ctx),
            11 => self.generate_lambda_expr(rng, depth, ctx),
            12 => self.generate_tuple_expr(rng, depth, ctx),
            13 => self.generate_list_expr(rng, depth, ctx),
            14 => self.generate_record_expr(rng, depth, ctx),
            15 => self.generate_range_expr(rng, depth, ctx),
            16 => self.generate_try_expr(rng, depth, ctx),
            17 if self.config.features.async_await => self.generate_async_expr(rng, depth, ctx),
            18 if self.config.features.async_await => self.generate_spawn_expr(rng, depth, ctx),
            _ => self.generate_simple_expr(rng, ctx),
        }
    }

    /// Generate a simple expression (for max depth or fallback)
    fn generate_simple_expr<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        if rng.random_bool(0.7) || ctx.variables.is_empty() {
            self.generate_literal(rng)
        } else {
            self.generate_identifier(rng, ctx)
        }
    }

    /// Generate a literal expression
    fn generate_literal<R: Rng>(&self, rng: &mut R) -> ArbitraryExpr {
        let lit = match rng.random_range(0..6) {
            0 => {
                let n = rng.random_range(
                    -self.config.complexity.max_int_value..=self.config.complexity.max_int_value,
                );
                LiteralValue::Int(n)
            }
            1 => {
                let n = rng.random::<f64>() * self.config.complexity.max_float_value * 2.0
                    - self.config.complexity.max_float_value;
                LiteralValue::Float(n)
            }
            2 => LiteralValue::Bool(rng.random()),
            3 => {
                let len = rng.random_range(0..self.config.complexity.max_string_length);
                let s: String = (0..len)
                    .map(|_| {
                        let idx = rng.random_range(0..62);
                        match idx {
                            0..=25 => (b'a' + idx as u8) as char,
                            26..=51 => (b'A' + (idx - 26) as u8) as char,
                            _ => (b'0' + (idx - 52) as u8) as char,
                        }
                    })
                    .collect();
                LiteralValue::Text(s)
            }
            4 => {
                let c = (b'a' + rng.random_range(0..26)) as char;
                LiteralValue::Char(c)
            }
            _ => LiteralValue::Unit,
        };

        ArbitraryExpr::new(lit.to_string(), ExprKind::Literal(lit), 0)
    }

    /// Generate an identifier expression
    pub fn generate_identifier<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let name = if ctx.variables.is_empty() || rng.random_bool(0.3) {
            ctx.fresh_variable()
        } else {
            ctx.variables.choose(rng).unwrap().clone()
        };

        ArbitraryExpr::new(name.clone(), ExprKind::Identifier(name), 0)
    }

    /// Generate a binary expression
    fn generate_binary_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let op = *BinaryOp::all().choose(rng).unwrap();
        let left = self.generate_expr(rng, depth + 1, ctx);
        let right = self.generate_expr(rng, depth + 1, ctx);

        let source = format!("({} {} {})", left.source, op.as_str(), right.source);

        ArbitraryExpr::new(
            source,
            ExprKind::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
            depth + 1,
        )
    }

    /// Generate a unary expression
    fn generate_unary_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let ops = [UnaryOp::Neg, UnaryOp::Not];
        let op = *ops.choose(rng).unwrap();
        let expr = self.generate_expr(rng, depth + 1, ctx);

        let source = format!("({}{})", op.as_str(), expr.source);

        ArbitraryExpr::new(
            source,
            ExprKind::Unary {
                op,
                expr: Box::new(expr),
            },
            depth + 1,
        )
    }

    /// Generate a function call expression
    fn generate_call_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let funcs = ["print", "len", "push", "pop", "abs", "min", "max", "sqrt"];
        let func = (*funcs.choose(rng).unwrap()).to_string();

        let num_args = rng.random_range(0..=3);
        let args: Vec<ArbitraryExpr> = (0..num_args)
            .map(|_| self.generate_expr(rng, depth + 1, ctx))
            .collect();

        let args_str = args
            .iter()
            .map(|a| a.source.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("{}({})", func, args_str);

        ArbitraryExpr::new(source, ExprKind::Call { func, args }, depth + 1)
    }

    /// Generate a method call expression
    fn generate_method_call<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let receiver = self.generate_expr(rng, depth + 1, ctx);
        let methods = ["len", "push", "pop", "get", "set", "clone", "map", "filter"];
        let method = (*methods.choose(rng).unwrap()).to_string();

        let num_args = rng.random_range(0..=2);
        let args: Vec<ArbitraryExpr> = (0..num_args)
            .map(|_| self.generate_expr(rng, depth + 1, ctx))
            .collect();

        let args_str = args
            .iter()
            .map(|a| a.source.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("{}.{}({})", receiver.source, method, args_str);

        ArbitraryExpr::new(
            source,
            ExprKind::MethodCall {
                receiver: Box::new(receiver),
                method,
                args,
            },
            depth + 1,
        )
    }

    /// Generate a field access expression
    fn generate_field_access<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let expr = self.generate_expr(rng, depth + 1, ctx);
        let fields = ["field", "value", "data", "inner", "0", "1", "2"];
        let field = (*fields.choose(rng).unwrap()).to_string();

        let source = format!("{}.{}", expr.source, field);

        ArbitraryExpr::new(
            source,
            ExprKind::FieldAccess {
                expr: Box::new(expr),
                field,
            },
            depth + 1,
        )
    }

    /// Generate an index expression
    fn generate_index_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let expr = self.generate_expr(rng, depth + 1, ctx);
        let index = self.generate_int_expr(rng, depth + 1, ctx);

        let source = format!("{}[{}]", expr.source, index.source);

        ArbitraryExpr::new(
            source,
            ExprKind::Index {
                expr: Box::new(expr),
                index: Box::new(index),
            },
            depth + 1,
        )
    }

    /// Generate an if expression
    fn generate_if_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let condition = self.generate_bool_expr(rng, depth + 1, ctx);
        let then_branch = self.generate_expr(rng, depth + 1, ctx);
        let else_branch = if rng.random_bool(0.7) {
            Some(self.generate_expr(rng, depth + 1, ctx))
        } else {
            None
        };

        let source = if let Some(ref else_br) = else_branch {
            format!(
                "if {} {{ {} }} else {{ {} }}",
                condition.source, then_branch.source, else_br.source
            )
        } else {
            format!("if {} {{ {} }}", condition.source, then_branch.source)
        };

        ArbitraryExpr::new(
            source,
            ExprKind::If {
                condition: Box::new(condition),
                then_branch: Box::new(then_branch),
                else_branch: else_branch.map(Box::new),
            },
            depth + 1,
        )
    }

    /// Generate a match expression
    fn generate_match_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let scrutinee = self.generate_expr(rng, depth + 1, ctx);
        let num_arms = rng.random_range(2..=self.config.complexity.max_match_arms);

        let mut arms = Vec::new();
        for i in 0..num_arms {
            let pattern = if i == num_arms - 1 {
                "_".to_string()
            } else {
                self.generate_pattern(rng, ctx)
            };
            let body = self.generate_expr(rng, depth + 1, ctx);
            arms.push((pattern, body));
        }

        let arms_str = arms
            .iter()
            .map(|(p, b)| format!("        {} => {}", p, b.source))
            .collect::<Vec<_>>()
            .join(",\n");

        let source = format!("match {} {{\n{}\n    }}", scrutinee.source, arms_str);

        ArbitraryExpr::new(
            source,
            ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
            depth + 1,
        )
    }

    /// Generate a block expression
    fn generate_block_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let num_exprs = rng.random_range(1..=3);
        let exprs: Vec<ArbitraryExpr> = (0..num_exprs)
            .map(|_| self.generate_expr(rng, depth + 1, ctx))
            .collect();

        let exprs_str = exprs
            .iter()
            .enumerate()
            .map(|(i, e)| {
                if i < exprs.len() - 1 {
                    format!("        {};", e.source)
                } else {
                    format!("        {}", e.source)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let source = format!("{{\n{}\n    }}", exprs_str);

        ArbitraryExpr::new(
            source,
            ExprKind::Block {
                exprs: exprs[..exprs.len() - 1].to_vec(),
                trailing: Some(Box::new(exprs.last().unwrap().clone())),
            },
            depth + 1,
        )
    }

    /// Generate a lambda expression
    fn generate_lambda_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let num_params = rng.random_range(1..=3);
        let params: Vec<String> = (0..num_params).map(|i| format!("x_{}", i)).collect();

        // Add params to context
        let mut inner_ctx = ctx.clone();
        inner_ctx.variables.extend(params.clone());

        let body = self.generate_expr(rng, depth + 1, &mut inner_ctx);

        let source = format!("|{}| {}", params.join(", "), body.source);

        ArbitraryExpr::new(
            source,
            ExprKind::Lambda {
                params,
                body: Box::new(body),
            },
            depth + 1,
        )
    }

    /// Generate a tuple expression
    fn generate_tuple_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let num_elements = rng.random_range(2..=4);
        let elements: Vec<ArbitraryExpr> = (0..num_elements)
            .map(|_| self.generate_expr(rng, depth + 1, ctx))
            .collect();

        let elements_str = elements
            .iter()
            .map(|e| e.source.clone())
            .collect::<Vec<_>>()
            .join(", ");

        let source = format!("({})", elements_str);

        ArbitraryExpr::new(source, ExprKind::Tuple { elements }, depth + 1)
    }

    /// Generate a list expression
    fn generate_list_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let num_elements = rng.random_range(0..=self.config.complexity.max_list_size.min(5));
        let elements: Vec<ArbitraryExpr> = (0..num_elements)
            .map(|_| self.generate_expr(rng, depth + 1, ctx))
            .collect();

        let elements_str = elements
            .iter()
            .map(|e| e.source.clone())
            .collect::<Vec<_>>()
            .join(", ");

        let source = format!("[{}]", elements_str);

        ArbitraryExpr::new(source, ExprKind::List { elements }, depth + 1)
    }

    /// Generate a record expression
    fn generate_record_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let num_fields = rng.random_range(1..=3);
        let mut fields = Vec::new();

        for i in 0..num_fields {
            let value = self.generate_expr(rng, depth + 1, ctx);
            fields.push((format!("field_{}", i), value));
        }

        let fields_str = fields
            .iter()
            .map(|(name, val)| format!("{}: {}", name, val.source))
            .collect::<Vec<_>>()
            .join(", ");

        let source = format!("{{ {} }}", fields_str);

        // Use Block kind for now since we don't have a specific Record kind
        ArbitraryExpr::new(
            source,
            ExprKind::Block {
                exprs: vec![],
                trailing: None,
            },
            depth + 1,
        )
    }

    /// Generate a range expression
    fn generate_range_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let has_start = rng.random_bool(0.9);
        let has_end = rng.random_bool(0.9);
        let inclusive = rng.random_bool(0.3);

        let start = if has_start {
            Some(self.generate_int_expr(rng, depth + 1, ctx))
        } else {
            None
        };

        let end = if has_end {
            Some(self.generate_int_expr(rng, depth + 1, ctx))
        } else {
            None
        };

        let source = match (&start, &end, inclusive) {
            (Some(s), Some(e), true) => format!("{}..={}", s.source, e.source),
            (Some(s), Some(e), false) => format!("{}..{}", s.source, e.source),
            (Some(s), None, _) => format!("{}..", s.source),
            (None, Some(e), true) => format!("..={}", e.source),
            (None, Some(e), false) => format!("..{}", e.source),
            (None, None, _) => "..".to_string(),
        };

        ArbitraryExpr::new(
            source,
            ExprKind::Range {
                start: start.map(Box::new),
                end: end.map(Box::new),
                inclusive,
            },
            depth + 1,
        )
    }

    /// Generate a try expression
    fn generate_try_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let expr = self.generate_expr(rng, depth + 1, ctx);
        let source = format!("{}?", expr.source);

        ArbitraryExpr::new(source, ExprKind::Try(Box::new(expr)), depth + 1)
    }

    /// Generate an async expression
    fn generate_async_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let expr = self.generate_expr(rng, depth + 1, ctx);
        let source = format!("async {{ {} }}", expr.source);

        ArbitraryExpr::new(source, ExprKind::Async(Box::new(expr)), depth + 1)
    }

    /// Generate a spawn expression
    fn generate_spawn_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        let expr = self.generate_expr(rng, depth + 1, ctx);
        let source = format!("spawn {{ {} }}", expr.source);

        ArbitraryExpr::new(source, ExprKind::Spawn(Box::new(expr)), depth + 1)
    }

    /// Generate an integer expression
    fn generate_int_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        if depth >= self.config.complexity.max_depth || rng.random_bool(0.5) {
            let n = rng.random_range(-1000..1000);
            return ArbitraryExpr::new(n.to_string(), ExprKind::Literal(LiteralValue::Int(n)), 0);
        }

        match rng.random_range(0..4) {
            0 => {
                // Binary arithmetic
                let left = self.generate_int_expr(rng, depth + 1, ctx);
                let right = self.generate_int_expr(rng, depth + 1, ctx);
                let op = *BinaryOp::arithmetic().choose(rng).unwrap();
                let source = format!("({} {} {})", left.source, op.as_str(), right.source);
                ArbitraryExpr::new(
                    source,
                    ExprKind::Binary {
                        op,
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    depth + 1,
                )
            }
            1 => {
                // Unary negation
                let expr = self.generate_int_expr(rng, depth + 1, ctx);
                let source = format!("(-{})", expr.source);
                ArbitraryExpr::new(
                    source,
                    ExprKind::Unary {
                        op: UnaryOp::Neg,
                        expr: Box::new(expr),
                    },
                    depth + 1,
                )
            }
            2 => {
                // If expression
                let cond = self.generate_bool_expr(rng, depth + 1, ctx);
                let then_br = self.generate_int_expr(rng, depth + 1, ctx);
                let else_br = self.generate_int_expr(rng, depth + 1, ctx);
                let source = format!(
                    "if {} {{ {} }} else {{ {} }}",
                    cond.source, then_br.source, else_br.source
                );
                ArbitraryExpr::new(
                    source,
                    ExprKind::If {
                        condition: Box::new(cond),
                        then_branch: Box::new(then_br.clone()),
                        else_branch: Some(Box::new(else_br)),
                    },
                    depth + 1,
                )
            }
            _ => {
                let n = rng.random_range(-1000..1000);
                ArbitraryExpr::new(n.to_string(), ExprKind::Literal(LiteralValue::Int(n)), 0)
            }
        }
    }

    /// Generate a float expression
    fn generate_float_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        if depth >= self.config.complexity.max_depth || rng.random_bool(0.6) {
            let n = rng.random::<f64>() * 200.0 - 100.0;
            return ArbitraryExpr::new(
                format!("{:.2}", n),
                ExprKind::Literal(LiteralValue::Float(n)),
                0,
            );
        }

        let left = self.generate_float_expr(rng, depth + 1, ctx);
        let right = self.generate_float_expr(rng, depth + 1, ctx);
        let op = *[BinaryOp::Add, BinaryOp::Sub, BinaryOp::Mul, BinaryOp::Div]
            .choose(rng)
            .unwrap();
        let source = format!("({} {} {})", left.source, op.as_str(), right.source);
        ArbitraryExpr::new(
            source,
            ExprKind::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
            depth + 1,
        )
    }

    /// Generate a boolean expression
    fn generate_bool_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        if depth >= self.config.complexity.max_depth || rng.random_bool(0.5) {
            let b = rng.random::<bool>();
            return ArbitraryExpr::new(b.to_string(), ExprKind::Literal(LiteralValue::Bool(b)), 0);
        }

        match rng.random_range(0..5) {
            0 | 1 => {
                // Comparison
                let left = self.generate_int_expr(rng, depth + 1, ctx);
                let right = self.generate_int_expr(rng, depth + 1, ctx);
                let op = *BinaryOp::comparison().choose(rng).unwrap();
                let source = format!("({} {} {})", left.source, op.as_str(), right.source);
                ArbitraryExpr::new(
                    source,
                    ExprKind::Binary {
                        op,
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    depth + 1,
                )
            }
            2 => {
                // Logical and/or
                let left = self.generate_bool_expr(rng, depth + 1, ctx);
                let right = self.generate_bool_expr(rng, depth + 1, ctx);
                let op = *BinaryOp::logical().choose(rng).unwrap();
                let source = format!("({} {} {})", left.source, op.as_str(), right.source);
                ArbitraryExpr::new(
                    source,
                    ExprKind::Binary {
                        op,
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    depth + 1,
                )
            }
            3 => {
                // Logical not
                let expr = self.generate_bool_expr(rng, depth + 1, ctx);
                let source = format!("(!{})", expr.source);
                ArbitraryExpr::new(
                    source,
                    ExprKind::Unary {
                        op: UnaryOp::Not,
                        expr: Box::new(expr),
                    },
                    depth + 1,
                )
            }
            _ => {
                let b = rng.random::<bool>();
                ArbitraryExpr::new(b.to_string(), ExprKind::Literal(LiteralValue::Bool(b)), 0)
            }
        }
    }

    /// Generate a text expression
    fn generate_text_expr<R: Rng>(
        &self,
        rng: &mut R,
        depth: usize,
        _ctx: &mut GenerationContext,
    ) -> ArbitraryExpr {
        if depth >= self.config.complexity.max_depth || rng.random_bool(0.7) {
            let len = rng.random_range(0..20);
            let s: String = (0..len)
                .map(|_| (b'a' + rng.random_range(0..26)) as char)
                .collect();
            return ArbitraryExpr::new(
                format!("\"{}\"", s),
                ExprKind::Literal(LiteralValue::Text(s)),
                0,
            );
        }

        // String concatenation
        let left = self.generate_text_expr(rng, depth + 1, _ctx);
        let right = self.generate_text_expr(rng, depth + 1, _ctx);
        let source = format!("({} + {})", left.source, right.source);
        ArbitraryExpr::new(
            source,
            ExprKind::Binary {
                op: BinaryOp::Add,
                left: Box::new(left),
                right: Box::new(right),
            },
            depth + 1,
        )
    }

    /// Generate a pattern
    fn generate_pattern<R: Rng>(&self, rng: &mut R, _ctx: &mut GenerationContext) -> String {
        match rng.random_range(0..5) {
            0 => "_".to_string(),
            1 => format!("x_{}", rng.random_range(0..100)),
            2 => rng.random_range(0..10).to_string(),
            3 => if rng.random() { "true" } else { "false" }.to_string(),
            _ => {
                let len = rng.random_range(1..5);
                let s: String = (0..len)
                    .map(|_| (b'a' + rng.random_range(0..26)) as char)
                    .collect();
                format!("\"{}\"", s)
            }
        }
    }
}

/// Context for tracking generation state
#[derive(Debug, Clone)]
pub struct GenerationContext {
    variables: Vec<String>,
    var_counter: usize,
}

impl GenerationContext {
    pub fn new() -> Self {
        Self {
            variables: Vec::new(),
            var_counter: 0,
        }
    }

    fn fresh_variable(&mut self) -> String {
        self.var_counter += 1;
        let name = format!("var_{}", self.var_counter);
        self.variables.push(name.clone());
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_generate_expression() {
        let config = GeneratorConfig::default();
        let generator = ExprGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..20 {
            let expr = generator.generate(&mut rng);
            assert!(!expr.source.is_empty());
        }
    }

    #[test]
    fn test_generate_typed() {
        let config = GeneratorConfig::default();
        let generator = ExprGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let int_expr = generator.generate_typed(&mut rng, "Int");
        assert!(!int_expr.source.is_empty());

        let bool_expr = generator.generate_typed(&mut rng, "Bool");
        assert!(!bool_expr.source.is_empty());

        let text_expr = generator.generate_typed(&mut rng, "Text");
        assert!(!text_expr.source.is_empty());
    }

    #[test]
    fn test_shrinking() {
        let binary = ArbitraryExpr::new(
            "(1 + 2)".to_string(),
            ExprKind::Binary {
                op: BinaryOp::Add,
                left: Box::new(ArbitraryExpr::new(
                    "1".to_string(),
                    ExprKind::Literal(LiteralValue::Int(1)),
                    0,
                )),
                right: Box::new(ArbitraryExpr::new(
                    "2".to_string(),
                    ExprKind::Literal(LiteralValue::Int(2)),
                    0,
                )),
            },
            1,
        );

        let shrunk = binary.shrink();
        assert!(!shrunk.is_empty());
        assert!(shrunk.iter().all(|s| s.complexity < binary.complexity));
    }

    #[test]
    fn test_literal_shrinking() {
        let lit = LiteralValue::Int(100);
        let shrunk = lit.shrink();
        assert!(shrunk.contains(&LiteralValue::Int(0)));
        assert!(shrunk.contains(&LiteralValue::Int(50)));
    }

    #[test]
    fn test_deterministic_with_seed() {
        let config = GeneratorConfig::default();
        let generator = ExprGenerator::new(config);

        let mut rng1 = ChaCha8Rng::seed_from_u64(12345);
        let mut rng2 = ChaCha8Rng::seed_from_u64(12345);

        let expr1 = generator.generate(&mut rng1);
        let expr2 = generator.generate(&mut rng2);

        assert_eq!(expr1.source, expr2.source);
    }
}
