//! RSL (Refinement Specification Language) Parser
//!
//! Parses contract# literals into structured ContractSpec AST nodes.
//! RSL syntax supports:
//! - Preconditions: `requires <expr>`
//! - Postconditions: `ensures <expr>`
//! - Invariants: `invariant <expr>`
//! - Special functions: `old(expr)`, `result`, `forall`, `exists`
//!
//! Contract literals use RSL embedded via `contract#"..."` syntax. Preconditions
//! become caller proof obligations, postconditions become callee proof obligations.
//! `old(expr)` refers to pre-state values; `result` refers to the return value.
//! Verified at compile time in `@verify(proof)` mode, at runtime in `@verify(runtime)`.

use verum_ast::{BinOp, Expr, ExprKind, Ident, Literal, Path, Span, UnOp};
use verum_common::{List, Maybe, Text};

/// A complete contract specification parsed from a contract# literal.
#[derive(Debug, Clone, PartialEq)]
pub struct ContractSpec {
    /// Preconditions (caller must ensure these)
    pub preconditions: List<RslClause>,

    /// Postconditions (function guarantees these)
    pub postconditions: List<RslClause>,

    /// Invariants (hold throughout execution)
    pub invariants: List<RslClause>,

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

    /// Get all clauses as a flat list.
    pub fn all_clauses(&self) -> List<&RslClause> {
        let mut clauses = List::new();
        for clause in self.preconditions.iter() {
            clauses.push(clause);
        }
        for clause in self.postconditions.iter() {
            clauses.push(clause);
        }
        for clause in self.invariants.iter() {
            clauses.push(clause);
        }
        clauses
    }
}

/// A single RSL clause (requires/ensures/invariant).
#[derive(Debug, Clone, PartialEq)]
pub struct RslClause {
    /// The kind of clause
    pub kind: RslClauseKind,

    /// The expression representing the constraint
    pub expr: Expr,

    /// Optional label for error messages
    pub label: Maybe<Text>,

    /// Source span for error reporting
    pub span: Span,
}

/// The kind of RSL clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RslClauseKind {
    /// Precondition (requires)
    Requires,

    /// Postcondition (ensures)
    Ensures,

    /// Invariant (invariant)
    Invariant,
}

impl RslClauseKind {
    /// Get the keyword for this clause kind.
    pub fn keyword(&self) -> &'static str {
        match self {
            RslClauseKind::Requires => "requires",
            RslClauseKind::Ensures => "ensures",
            RslClauseKind::Invariant => "invariant",
        }
    }
}

/// Parser for RSL contract specifications.
#[derive(Debug)]
pub struct RslParser {
    /// Input text
    input: Text,

    /// Current position in input
    pos: usize,

    /// Source span for error reporting
    span: Span,
}

impl RslParser {
    /// Create a new RSL parser for the given input.
    pub fn new(input: Text, span: Span) -> Self {
        Self {
            input,
            pos: 0,
            span,
        }
    }

    /// Parse a contract specification.
    pub fn parse(&mut self) -> Result<ContractSpec, RslParseError> {
        let mut spec = ContractSpec::new(self.span);

        // Skip leading whitespace
        self.skip_whitespace();

        // Parse clauses until EOF
        while !self.is_eof() {
            let clause = self.parse_clause()?;

            match clause.kind {
                RslClauseKind::Requires => spec.preconditions.push(clause),
                RslClauseKind::Ensures => spec.postconditions.push(clause),
                RslClauseKind::Invariant => spec.invariants.push(clause),
            }

            self.skip_whitespace();
        }

        Ok(spec)
    }

    /// Parse a single clause (requires/ensures/invariant).
    fn parse_clause(&mut self) -> Result<RslClause, RslParseError> {
        // Parse clause keyword
        let kind = if self.consume_keyword("requires") {
            RslClauseKind::Requires
        } else if self.consume_keyword("ensures") {
            RslClauseKind::Ensures
        } else if self.consume_keyword("invariant") {
            RslClauseKind::Invariant
        } else {
            return Err(RslParseError::ExpectedKeyword {
                expected: "requires, ensures, or invariant".to_string(),
                found: self.peek_word().unwrap_or_default().to_string(),
                pos: self.pos,
            });
        };

        self.skip_whitespace();

        // Parse the constraint expression
        let expr = self.parse_expr()?;

        // Optionally consume semicolon
        self.skip_whitespace();
        if self.peek_char() == Some(';') {
            self.advance();
        }

        Ok(RslClause {
            kind,
            expr,
            label: Maybe::None,
            span: self.span,
        })
    }

    /// Parse an expression.
    fn parse_expr(&mut self) -> Result<Expr, RslParseError> {
        // Implication has lowest precedence
        self.parse_implication()
    }

    /// Parse logical OR expression.
    fn parse_logical_or(&mut self) -> Result<Expr, RslParseError> {
        let mut left = self.parse_logical_and()?;

        while self.peek_op() == Some("||") {
            self.consume_op("||");
            self.skip_whitespace();
            let right = self.parse_logical_and()?;
            left = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Or,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                self.span,
            );
        }

        Ok(left)
    }

    /// Parse logical AND expression.
    fn parse_logical_and(&mut self) -> Result<Expr, RslParseError> {
        let mut left = self.parse_comparison()?;

        while self.peek_op() == Some("&&") {
            self.consume_op("&&");
            self.skip_whitespace();
            let right = self.parse_comparison()?;
            left = Expr::new(
                ExprKind::Binary {
                    op: BinOp::And,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                self.span,
            );
        }

        Ok(left)
    }

    /// Parse comparison expression (==, !=, <, <=, >, >=).
    fn parse_comparison(&mut self) -> Result<Expr, RslParseError> {
        // Comparison has higher precedence than implication
        let mut left = self.parse_additive()?;

        if let Some(op_str) = self.peek_comparison_op() {
            let op = match op_str {
                "==" => BinOp::Eq,
                "!=" => BinOp::Ne,
                "<" => BinOp::Lt,
                "<=" => BinOp::Le,
                ">" => BinOp::Gt,
                ">=" => BinOp::Ge,
                _ => unreachable!(),
            };

            self.consume_op(op_str);
            self.skip_whitespace();
            let right = self.parse_additive()?;

            left = Expr::new(
                ExprKind::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                self.span,
            );
        }

        Ok(left)
    }

    /// Parse implication expression (=>).
    /// Implication has lower precedence than logical operators.
    fn parse_implication(&mut self) -> Result<Expr, RslParseError> {
        let mut left = self.parse_logical_or()?;

        if self.peek_op() == Some("=>") || self.peek_op() == Some("==>") {
            let op_str = if self.peek_op() == Some("==>") {
                "==>"
            } else {
                "=>"
            };
            self.consume_op(op_str);
            self.skip_whitespace();
            let right = self.parse_implication()?;

            // Convert implication to: !left || right
            left = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Or,
                    left: Box::new(Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Not,
                            expr: Box::new(left),
                        },
                        self.span,
                    )),
                    right: Box::new(right),
                },
                self.span,
            );
        }

        Ok(left)
    }

    /// Parse additive expression (+, -).
    fn parse_additive(&mut self) -> Result<Expr, RslParseError> {
        let mut left = self.parse_multiplicative()?;

        while let Some(op_str) = self.peek_additive_op() {
            let op = match op_str {
                "+" => BinOp::Add,
                "-" => BinOp::Sub,
                _ => unreachable!(),
            };

            self.consume_op(op_str);
            self.skip_whitespace();
            let right = self.parse_multiplicative()?;

            left = Expr::new(
                ExprKind::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                self.span,
            );
        }

        Ok(left)
    }

    /// Parse multiplicative expression (*, /, %).
    fn parse_multiplicative(&mut self) -> Result<Expr, RslParseError> {
        let mut left = self.parse_unary()?;

        while let Some(op_str) = self.peek_multiplicative_op() {
            let op = match op_str {
                "*" => BinOp::Mul,
                "/" => BinOp::Div,
                "%" => BinOp::Rem,
                _ => unreachable!(),
            };

            self.consume_op(op_str);
            self.skip_whitespace();
            let right = self.parse_unary()?;

            left = Expr::new(
                ExprKind::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                self.span,
            );
        }

        Ok(left)
    }

    /// Parse unary expression (!, -).
    fn parse_unary(&mut self) -> Result<Expr, RslParseError> {
        if let Some(op_str) = self.peek_unary_op() {
            let op = match op_str {
                "!" => UnOp::Not,
                "-" => UnOp::Neg,
                _ => unreachable!(),
            };

            self.consume_op(op_str);
            self.skip_whitespace();
            let expr = self.parse_unary()?;

            Ok(Expr::new(
                ExprKind::Unary {
                    op,
                    expr: Box::new(expr),
                },
                self.span,
            ))
        } else {
            self.parse_postfix()
        }
    }

    /// Parse postfix expression (function calls, field access, indexing).
    fn parse_postfix(&mut self) -> Result<Expr, RslParseError> {
        let mut expr = self.parse_primary()?;

        loop {
            self.skip_whitespace();

            if self.peek_char() == Some('(') {
                // Function call
                self.advance(); // consume '('
                self.skip_whitespace();

                let mut args = List::new();
                while self.peek_char() != Some(')') {
                    args.push(self.parse_expr()?);
                    self.skip_whitespace();

                    if self.peek_char() == Some(',') {
                        self.advance();
                        self.skip_whitespace();
                    } else {
                        break;
                    }
                }

                if self.peek_char() != Some(')') {
                    return Err(RslParseError::Expected {
                        expected: ')'.to_string(),
                        found: self.peek_char().unwrap_or('\0'),
                        pos: self.pos,
                    });
                }
                self.advance(); // consume ')'

                expr = Expr::new(
                    ExprKind::Call {
                        func: Box::new(expr),
                        type_args: List::new(),
                        args,
                    },
                    self.span,
                );
            } else if self.peek_char() == Some('.') {
                // Field access or method call
                self.advance(); // consume '.'
                let field_name = self.parse_identifier()?;
                self.skip_whitespace();

                if self.peek_char() == Some('(') {
                    // Method call
                    self.advance(); // consume '('
                    self.skip_whitespace();

                    let mut args = List::new();
                    while self.peek_char() != Some(')') {
                        args.push(self.parse_expr()?);
                        self.skip_whitespace();

                        if self.peek_char() == Some(',') {
                            self.advance();
                            self.skip_whitespace();
                        } else {
                            break;
                        }
                    }

                    if self.peek_char() != Some(')') {
                        return Err(RslParseError::Expected {
                            expected: ')'.to_string(),
                            found: self.peek_char().unwrap_or('\0'),
                            pos: self.pos,
                        });
                    }
                    self.advance(); // consume ')'

                    expr = Expr::new(
                        ExprKind::MethodCall {
                            receiver: Box::new(expr),
                            method: Ident::new(field_name, self.span),
                            type_args: List::new(),
                            args,
                        },
                        self.span,
                    );
                } else {
                    // Field access
                    expr = Expr::new(
                        ExprKind::Field {
                            expr: Box::new(expr),
                            field: Ident::new(field_name, self.span),
                        },
                        self.span,
                    );
                }
            } else if self.peek_char() == Some('[') {
                // Index expression
                self.advance(); // consume '['
                self.skip_whitespace();
                let index = self.parse_expr()?;
                self.skip_whitespace();

                if self.peek_char() != Some(']') {
                    return Err(RslParseError::Expected {
                        expected: ']'.to_string(),
                        found: self.peek_char().unwrap_or('\0'),
                        pos: self.pos,
                    });
                }
                self.advance(); // consume ']'

                expr = Expr::new(
                    ExprKind::Index {
                        expr: Box::new(expr),
                        index: Box::new(index),
                    },
                    self.span,
                );
            } else {
                break;
            }
        }

        Ok(expr)
    }

    /// Parse primary expression (literals, identifiers, parenthesized expressions).
    fn parse_primary(&mut self) -> Result<Expr, RslParseError> {
        self.skip_whitespace();

        // Parenthesized expression
        if self.peek_char() == Some('(') {
            self.advance(); // consume '('
            self.skip_whitespace();
            let expr = self.parse_expr()?;
            self.skip_whitespace();

            if self.peek_char() != Some(')') {
                return Err(RslParseError::Expected {
                    expected: ')'.to_string(),
                    found: self.peek_char().unwrap_or('\0'),
                    pos: self.pos,
                });
            }
            self.advance(); // consume ')'

            return Ok(Expr::new(ExprKind::Paren(Box::new(expr)), self.span));
        }

        // Number literal
        if let Some(c) = self.peek_char()
            && c.is_ascii_digit()
        {
            return self.parse_number();
        }

        // Identifier or keyword
        if let Some(word) = self.peek_word() {
            // Special RSL keywords
            match word.as_str() {
                "true" => {
                    self.consume_word(word.as_str());
                    return Ok(Expr::literal(Literal::bool(true, self.span)));
                }
                "false" => {
                    self.consume_word(word.as_str());
                    return Ok(Expr::literal(Literal::bool(false, self.span)));
                }
                "old" => {
                    self.consume_word(word.as_str());
                    self.skip_whitespace();

                    if self.peek_char() != Some('(') {
                        return Err(RslParseError::Expected {
                            expected: '('.to_string(),
                            found: self.peek_char().unwrap_or('\0'),
                            pos: self.pos,
                        });
                    }
                    self.advance(); // consume '('
                    self.skip_whitespace();

                    let expr = self.parse_expr()?;
                    self.skip_whitespace();

                    if self.peek_char() != Some(')') {
                        return Err(RslParseError::Expected {
                            expected: ')'.to_string(),
                            found: self.peek_char().unwrap_or('\0'),
                            pos: self.pos,
                        });
                    }
                    self.advance(); // consume ')'

                    // Represent old(expr) as a special function call
                    let old_func =
                        Expr::path(Path::single(Ident::new(Text::from("old"), self.span)));
                    return Ok(Expr::new(
                        ExprKind::Call {
                            func: Box::new(old_func),
                            type_args: List::new(),
                            args: {
                                let mut args = List::new();
                                args.push(expr);
                                args
                            },
                        },
                        self.span,
                    ));
                }
                "result" => {
                    self.consume_word(word.as_str());
                    return Ok(Expr::path(Path::single(Ident::new(
                        Text::from("result"),
                        self.span,
                    ))));
                }
                "forall" | "exists" => {
                    // Quantifiers - for now, parse as function calls
                    // Full support requires quantified formulas in SMT
                    let quant_name = word.clone();
                    self.consume_word(word.as_str());
                    self.skip_whitespace();

                    if self.peek_char() != Some('(') {
                        return Err(RslParseError::Expected {
                            expected: '('.to_string(),
                            found: self.peek_char().unwrap_or('\0'),
                            pos: self.pos,
                        });
                    }
                    self.advance(); // consume '('
                    self.skip_whitespace();

                    // Parse arguments
                    let mut args = List::new();
                    while self.peek_char() != Some(')') {
                        args.push(self.parse_expr()?);
                        self.skip_whitespace();

                        if self.peek_char() == Some(',') {
                            self.advance();
                            self.skip_whitespace();
                        } else {
                            break;
                        }
                    }

                    if self.peek_char() != Some(')') {
                        return Err(RslParseError::Expected {
                            expected: ')'.to_string(),
                            found: self.peek_char().unwrap_or('\0'),
                            pos: self.pos,
                        });
                    }
                    self.advance(); // consume ')'

                    let quant_func = Expr::path(Path::single(Ident::new(quant_name, self.span)));
                    return Ok(Expr::new(
                        ExprKind::Call {
                            func: Box::new(quant_func),
                            type_args: List::new(),
                            args,
                        },
                        self.span,
                    ));
                }
                _ => {}
            }

            // Regular identifier
            let ident = self.parse_identifier()?;
            return Ok(Expr::path(Path::single(Ident::new(ident, self.span))));
        }

        Err(RslParseError::UnexpectedToken {
            found: self.peek_char().unwrap_or('\0').to_string(),
            pos: self.pos,
        })
    }

    /// Parse a number literal (integer or float).
    fn parse_number(&mut self) -> Result<Expr, RslParseError> {
        let start = self.pos;
        let mut has_dot = false;

        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() {
                self.advance();
            } else if c == '.' && !has_dot {
                has_dot = true;
                self.advance();
            } else {
                break;
            }
        }

        let num_str = &self.input[start..self.pos];

        if has_dot {
            // Float
            let value = num_str
                .parse::<f64>()
                .map_err(|_| RslParseError::InvalidNumber {
                    text: num_str.to_string(),
                    pos: start,
                })?;
            Ok(Expr::literal(Literal::float(value, self.span)))
        } else {
            // Integer
            let value = num_str
                .parse::<i128>()
                .map_err(|_| RslParseError::InvalidNumber {
                    text: num_str.to_string(),
                    pos: start,
                })?;
            Ok(Expr::literal(Literal::int(value, self.span)))
        }
    }

    /// Parse an identifier.
    fn parse_identifier(&mut self) -> Result<Text, RslParseError> {
        let start = self.pos;

        if let Some(c) = self.peek_char() {
            if !c.is_alphabetic() && c != '_' {
                return Err(RslParseError::ExpectedIdentifier {
                    found: c,
                    pos: self.pos,
                });
            }
            self.advance();
        } else {
            return Err(RslParseError::UnexpectedEof { pos: self.pos });
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

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek_char() {
            if c.is_whitespace() {
                self.advance();
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
        let start = self.pos;

        if self.input[self.pos..].starts_with(keyword) {
            let next_pos = self.pos + keyword.len();

            // Make sure it's not part of a longer identifier
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

        self.pos = start;
        false
    }

    fn peek_op(&self) -> Option<&'static str> {
        let ops = ["==", "!=", "<=", ">=", "&&", "||", "=>", "==>"];

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
        } else if self.input[self.pos..].starts_with('-') {
            // Make sure it's not unary minus (followed by space or digit)
            if self.pos + 1 < self.input.len() {
                let next = self.input[self.pos + 1..].chars().next().unwrap();
                if next.is_whitespace() || !next.is_ascii_digit() {
                    Some("-")
                } else {
                    None
                }
            } else {
                None
            }
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

    fn peek_unary_op(&self) -> Option<&'static str> {
        if self.input[self.pos..].starts_with('!') {
            Some("!")
        } else if self.input[self.pos..].starts_with('-') {
            Some("-")
        } else {
            None
        }
    }

    fn consume_op(&mut self, op: &str) {
        self.pos += op.len();
    }
}

/// Errors that can occur during RSL parsing.
#[derive(Debug, thiserror::Error)]
pub enum RslParseError {
    /// Expected a specific keyword
    #[error("expected {expected}, found '{found}' at position {pos}")]
    ExpectedKeyword {
        expected: String,
        found: String,
        pos: usize,
    },

    /// Expected a specific character
    #[error("expected {expected}, found '{found}' at position {pos}")]
    Expected {
        expected: String,
        found: char,
        pos: usize,
    },

    /// Expected an identifier
    #[error("expected identifier, found '{found}' at position {pos}")]
    ExpectedIdentifier { found: char, pos: usize },

    /// Unexpected token
    #[error("unexpected token '{found}' at position {pos}")]
    UnexpectedToken { found: String, pos: usize },

    /// Unexpected end of file
    #[error("unexpected end of file at position {pos}")]
    UnexpectedEof { pos: usize },

    /// Invalid number literal
    #[error("invalid number literal '{text}' at position {pos}")]
    InvalidNumber { text: String, pos: usize },
}
