//! Quote - Code generation infrastructure for meta functions
//!
//! This module provides the `TokenStream` type and `quote!()` macro
//! for generating Verum code at compile-time.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use verum_ast::{Item, Span, expr::{Expr, RecoverBody}, pattern::Pattern, stmt::Stmt, ty::Type};
use verum_lexer::{FloatLiteral, IntegerLiteral, Token, TokenKind};
use verum_fast_parser::VerumParser;
use verum_common::{List, Map, Maybe, Text};

/// A stream of tokens representing generated code
///
/// TokenStream is the fundamental type for code generation in meta functions.
/// It can be parsed back into AST nodes for insertion into the compilation.
#[derive(Debug, Clone)]
pub struct TokenStream {
    /// The tokens in this stream
    tokens: List<Token>,

    /// Source span for error reporting
    span: Maybe<Span>,
}

impl TokenStream {
    /// Create a new empty token stream
    pub fn new() -> Self {
        Self {
            tokens: List::new(),
            span: Maybe::None,
        }
    }

    /// Create a token stream from a list of tokens
    pub fn from_tokens(tokens: List<Token>) -> Self {
        Self {
            tokens,
            span: Maybe::None,
        }
    }

    /// Create a token stream with a source span
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Maybe::Some(span);
        self
    }

    /// Push a token onto the stream
    pub fn push(&mut self, token: Token) {
        self.tokens.push(token);
    }

    /// Extend the stream with tokens from another stream
    pub fn extend(&mut self, other: TokenStream) {
        for token in other.tokens {
            self.tokens.push(token);
        }
    }

    /// Get the tokens in this stream
    pub fn tokens(&self) -> &List<Token> {
        &self.tokens
    }

    /// Get the number of tokens in the stream
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// Check if the stream is empty
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Parse the token stream as an expression
    ///
    /// This method converts the token stream into an AST expression node.
    /// The parser is invoked directly on the tokens, enabling meta-functions
    /// to generate code at compile-time.
    ///
    /// # Errors
    ///
    /// Returns `ParseError::EmptyTokenStream` if the token stream is empty.
    /// Returns `ParseError::ParseFailed` if the tokens don't form a valid expression.
    /// Returns `ParseError::UnconsumedTokens` if tokens remain after parsing.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let ts = quote! { 1 + 2 };
    /// let expr = ts.parse_as_expr().unwrap();
    /// assert!(matches!(expr.kind, ExprKind::Binary { .. }));
    /// ```
    pub fn parse_as_expr(&self) -> Result<Expr, ParseError> {
        if self.tokens.is_empty() {
            return Err(ParseError::EmptyTokenStream);
        }

        // Parse directly from tokens using the parser's internal API
        // Convert List to List for parser
        let tokens_list: List<_> = self.tokens.iter().cloned().collect();
        self.parse_with(|parser| {
            parser
                .parse_expr_tokens(&tokens_list)
                .map_err(|e| Text::from(e.as_str()))
        })
    }

    /// Parse the token stream as a type
    ///
    /// This method converts the token stream into an AST type node.
    ///
    /// # Errors
    ///
    /// Returns `ParseError::EmptyTokenStream` if the token stream is empty.
    /// Returns `ParseError::ParseFailed` if the tokens don't form a valid type.
    /// Returns `ParseError::UnconsumedTokens` if tokens remain after parsing.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let ts = quote! { List<Int> };
    /// let ty = ts.parse_as_type().unwrap();
    /// ```
    pub fn parse_as_type(&self) -> Result<Type, ParseError> {
        if self.tokens.is_empty() {
            return Err(ParseError::EmptyTokenStream);
        }

        // Parse directly from tokens
        let tokens_list: List<_> = self.tokens.iter().cloned().collect();
        self.parse_with(|parser| {
            parser
                .parse_type_tokens(&tokens_list)
                .map_err(|e| Text::from(e.as_str()))
        })
    }

    /// Parse the token stream as an item (function, type, etc.)
    ///
    /// This method converts the token stream into an AST item node (function declaration,
    /// type definition, protocol, etc.).
    ///
    /// # Errors
    ///
    /// Returns `ParseError::EmptyTokenStream` if the token stream is empty.
    /// Returns `ParseError::ParseFailed` if the tokens don't form a valid item.
    /// Returns `ParseError::UnconsumedTokens` if tokens remain after parsing.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let ts = quote! {
    ///     fn add(a: Int, b: Int) -> Int {
    ///         a + b
    ///     }
    /// };
    /// let item = ts.parse_as_item().unwrap();
    /// assert!(matches!(item, Item::Function { .. }));
    /// ```
    pub fn parse_as_item(&self) -> Result<Item, ParseError> {
        if self.tokens.is_empty() {
            return Err(ParseError::EmptyTokenStream);
        }

        // Parse directly from tokens
        let tokens_list: List<_> = self.tokens.iter().cloned().collect();
        self.parse_with(|parser| {
            parser
                .parse_item_tokens(&tokens_list)
                .map_err(|e| Text::from(e.as_str()))
        })
    }

    /// Parse the token stream as multiple items (for staged metaprogramming).
    ///
    /// This method parses all items in the token stream. It's used when a meta
    /// function generates multiple items (e.g., a type definition and several
    /// functions implementing it).
    ///
    /// # Returns
    ///
    /// An empty list if the token stream is empty, or a list of all parsed items.
    ///
    /// # Errors
    ///
    /// Returns `ParseError::ParseFailed` if the tokens don't form valid items.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let ts = quote! {
    ///     type Point is { x: Float, y: Float };
    ///
    ///     fn distance(a: Point, b: Point) -> Float {
    ///         let dx = b.x - a.x;
    ///         let dy = b.y - a.y;
    ///         (dx * dx + dy * dy).sqrt()
    ///     }
    /// };
    /// let items = ts.parse_as_items().unwrap();
    /// assert_eq!(items.len(), 2);
    /// ```
    pub fn parse_as_items(&self) -> Result<List<Item>, ParseError> {
        if self.tokens.is_empty() {
            return Ok(List::new());
        }

        let tokens_list: List<_> = self.tokens.iter().cloned().collect();
        self.parse_with(|parser| {
            parser
                .parse_items_tokens(&tokens_list)
                .map_err(|e| Text::from(e.as_str()))
        })
    }

    /// Internal helper to parse tokens with proper error handling
    fn parse_with<T, F>(&self, parse_fn: F) -> Result<T, ParseError>
    where
        F: FnOnce(&VerumParser) -> Result<T, Text>,
    {
        let parser = VerumParser::new();
        parse_fn(&parser).map_err(|msg| ParseError::ParseFailed(msg))
    }

    /// Convert a string into a token stream
    ///
    /// This is useful for creating simple token streams from code strings.
    pub fn from_str(source: &str, file_id: verum_ast::FileId) -> Result<Self, ParseError> {
        use verum_lexer::Lexer;

        let lexer = Lexer::new(source, file_id);
        let tokens: List<Token> = lexer.filter_map(|r| r.ok()).collect();

        Ok(Self::from_tokens(tokens))
    }
}

impl Default for TokenStream {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for types that can be converted to tokens
pub trait ToTokens {
    /// Write this value as tokens to a token stream
    fn to_tokens(&self, stream: &mut TokenStream);

    /// Convert to a token stream
    fn into_token_stream(&self) -> TokenStream {
        let mut stream = TokenStream::new();
        self.to_tokens(&mut stream);
        stream
    }
}

impl ToTokens for Expr {
    fn to_tokens(&self, stream: &mut TokenStream) {
        use verum_ast::BinOp;
        use verum_ast::UnOp;
        use verum_ast::expr::ExprKind;

        match &self.kind {
            ExprKind::Literal(lit) => {
                use verum_ast::LiteralKind;
                match &lit.kind {
                    LiteralKind::Bool(b) => {
                        let kind = if *b {
                            TokenKind::True
                        } else {
                            TokenKind::False
                        };
                        stream.push(Token::new(kind, self.span));
                    }
                    LiteralKind::Int(i) => {
                        stream.push(Token::new(
                            TokenKind::Integer(IntegerLiteral {
                                raw_value: i.value.to_string().into(),
                                base: 10,
                                suffix: None,
                            }),
                            self.span,
                        ));
                    }
                    LiteralKind::Float(f) => {
                        stream.push(Token::new(
                            TokenKind::Float(FloatLiteral {
                                value: f.value,
                                suffix: None,
                                raw: format!("{}", f.value).into(),
                            }),
                            self.span,
                        ));
                    }
                    LiteralKind::Text(s) => {
                        stream.push(Token::new(
                            TokenKind::Text(s.as_str().to_string().into()),
                            self.span,
                        ));
                    }
                    LiteralKind::Char(c) => {
                        stream.push(Token::new(TokenKind::Char(*c), self.span));
                    }
                    LiteralKind::ByteChar(b) => {
                        stream.push(Token::new(TokenKind::ByteChar(*b), self.span));
                    }
                    LiteralKind::ByteString(bytes) => {
                        stream.push(Token::new(TokenKind::ByteString(bytes.clone()), self.span));
                    }
                    LiteralKind::Tagged { tag, content } => {
                        stream.push(Token::new(
                            TokenKind::Ident(tag.as_str().to_string().into()),
                            self.span,
                        ));
                        stream.push(Token::new(TokenKind::Hash, self.span));
                        stream.push(Token::new(
                            TokenKind::Text(content.as_str().to_string().into()),
                            self.span,
                        ));
                    }
                    LiteralKind::InterpolatedString(interp) => {
                        stream.push(Token::new(
                            TokenKind::Ident(interp.prefix.as_str().to_string().into()),
                            self.span,
                        ));
                        stream.push(Token::new(
                            TokenKind::Text(interp.content.as_str().to_string().into()),
                            self.span,
                        ));
                    }
                    LiteralKind::Contract(content) => {
                        stream.push(Token::new(
                            TokenKind::Ident("contract".into()),
                            self.span,
                        ));
                        stream.push(Token::new(TokenKind::Hash, self.span));
                        stream.push(Token::new(
                            TokenKind::Text(content.as_str().to_string().into()),
                            self.span,
                        ));
                    }
                    LiteralKind::Composite(comp) => {
                        stream.push(Token::new(
                            TokenKind::Ident(comp.tag.as_str().to_string().into()),
                            self.span,
                        ));
                        stream.push(Token::new(TokenKind::Hash, self.span));
                        stream.push(Token::new(
                            TokenKind::Text(comp.content.as_str().to_string().into()),
                            self.span,
                        ));
                    }
                    LiteralKind::ContextAdaptive(ctx_lit) => {
                        stream.push(Token::new(
                            TokenKind::Ident(ctx_lit.raw.as_str().to_string().into()),
                            self.span,
                        ));
                    }
                }
            }

            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    stream.push(Token::new(
                        TokenKind::Ident(ident.as_str().to_string().into()),
                        self.span,
                    ));
                } else {
                    for (i, segment) in path.segments.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Dot, self.span));
                        }
                        if let verum_ast::ty::PathSegment::Name(ident) = segment {
                            stream.push(Token::new(
                                TokenKind::Ident(ident.as_str().to_string().into()),
                                self.span,
                            ));
                        }
                    }
                }
            }

            ExprKind::Binary { op, left, right } => {
                left.to_tokens(stream);
                let op_kind = match op {
                    BinOp::Add => TokenKind::Plus,
                    BinOp::Sub => TokenKind::Minus,
                    BinOp::Mul => TokenKind::Star,
                    BinOp::Div => TokenKind::Slash,
                    BinOp::Rem => TokenKind::Percent,
                    BinOp::Eq => TokenKind::EqEq,
                    BinOp::Ne => TokenKind::BangEq,
                    BinOp::Lt => TokenKind::Lt,
                    BinOp::Le => TokenKind::LtEq,
                    BinOp::Gt => TokenKind::Gt,
                    BinOp::Ge => TokenKind::GtEq,
                    BinOp::And => TokenKind::AmpersandAmpersand,
                    BinOp::Or => TokenKind::PipePipe,
                    BinOp::BitAnd => TokenKind::Ampersand,
                    BinOp::BitOr => TokenKind::Pipe,
                    BinOp::BitXor => TokenKind::Ident("^".into()),
                    BinOp::Shl => TokenKind::Ident("<<".into()),
                    BinOp::Shr => TokenKind::Ident(">>".into()),
                    BinOp::Assign => TokenKind::Eq,
                    _ => TokenKind::Plus,
                };
                stream.push(Token::new(op_kind, self.span));
                right.to_tokens(stream);
            }

            ExprKind::Unary { op, expr } => {
                let op_kind = match op {
                    UnOp::Neg => TokenKind::Minus,
                    UnOp::Not => TokenKind::Bang,
                    UnOp::BitNot => TokenKind::Ident("~".into()),
                    UnOp::Deref => TokenKind::Star,
                    UnOp::Ref => TokenKind::Ampersand,
                    UnOp::RefMut => TokenKind::Ampersand,
                    UnOp::RefChecked => TokenKind::Ampersand,
                    UnOp::RefCheckedMut => TokenKind::Ampersand,
                    UnOp::RefUnsafe => TokenKind::Ampersand,
                    UnOp::RefUnsafeMut => TokenKind::Ampersand,
                    UnOp::Own => TokenKind::Ident("%".into()),
                    UnOp::OwnMut => TokenKind::Ident("%".into()),
                };
                stream.push(Token::new(op_kind, self.span));
                expr.to_tokens(stream);
            }

            ExprKind::Call { func, args, .. } => {
                func.to_tokens(stream);
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    arg.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
            }

            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                receiver.to_tokens(stream);
                stream.push(Token::new(TokenKind::Dot, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(method.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    arg.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
            }

            ExprKind::Field { expr, field } => {
                expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::Dot, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(field.as_str().to_string().into()),
                    self.span,
                ));
            }

            ExprKind::Index { expr, index } => {
                expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::LBracket, self.span));
                index.to_tokens(stream);
                stream.push(Token::new(TokenKind::RBracket, self.span));
            }

            ExprKind::Tuple(exprs) => {
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, expr) in exprs.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    expr.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
            }

            ExprKind::Block(block) => {
                block.to_tokens(stream);
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                stream.push(Token::new(TokenKind::If, self.span));
                condition.to_tokens(stream);
                then_branch.to_tokens(stream);
                if let Some(else_expr) = else_branch {
                    stream.push(Token::new(TokenKind::Else, self.span));
                    else_expr.to_tokens(stream);
                }
            }

            ExprKind::Match { expr, arms } => {
                stream.push(Token::new(TokenKind::Match, self.span));
                expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::LBrace, self.span));
                for arm in arms.iter() {
                    arm.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            ExprKind::Loop { label, body, invariants } => {
                // Optional label: 'loop_name:
                if let Some(lbl) = label {
                    stream.push(Token::new(
                        TokenKind::Lifetime(format!("'{}", lbl.as_str()).into()),
                        self.span,
                    ));
                    stream.push(Token::new(TokenKind::Colon, self.span));
                }

                stream.push(Token::new(TokenKind::Loop, self.span));

                // Loop invariant annotations
                for inv in invariants.iter() {
                    stream.push(Token::new(TokenKind::Invariant, self.span));
                    inv.to_tokens(stream);
                }

                body.to_tokens(stream);
            }

            ExprKind::While {
                label,
                condition,
                body,
                invariants,
                decreases,
            } => {
                // Optional label: 'loop_name:
                if let Some(lbl) = label {
                    stream.push(Token::new(
                        TokenKind::Lifetime(format!("'{}", lbl.as_str()).into()),
                        self.span,
                    ));
                    stream.push(Token::new(TokenKind::Colon, self.span));
                }

                stream.push(Token::new(TokenKind::While, self.span));
                condition.to_tokens(stream);

                // Loop invariant annotations
                for inv in invariants.iter() {
                    stream.push(Token::new(TokenKind::Invariant, self.span));
                    inv.to_tokens(stream);
                }

                // Decreases annotations for termination
                for dec in decreases.iter() {
                    stream.push(Token::new(TokenKind::Decreases, self.span));
                    dec.to_tokens(stream);
                }

                body.to_tokens(stream);
            }

            ExprKind::For {
                label,
                pattern,
                iter,
                body,
                invariants,
                decreases,
            } => {
                // Optional label: 'loop_name:
                if let Some(lbl) = label {
                    stream.push(Token::new(
                        TokenKind::Lifetime(format!("'{}", lbl.as_str()).into()),
                        self.span,
                    ));
                    stream.push(Token::new(TokenKind::Colon, self.span));
                }

                stream.push(Token::new(TokenKind::For, self.span));
                pattern.to_tokens(stream);
                stream.push(Token::new(TokenKind::In, self.span));
                iter.to_tokens(stream);

                // Loop invariant annotations
                for inv in invariants.iter() {
                    stream.push(Token::new(TokenKind::Invariant, self.span));
                    inv.to_tokens(stream);
                }

                // Decreases annotations for termination
                for dec in decreases.iter() {
                    stream.push(Token::new(TokenKind::Decreases, self.span));
                    dec.to_tokens(stream);
                }

                body.to_tokens(stream);
            }

            ExprKind::ForAwait {
                label,
                pattern,
                async_iterable,
                body,
                invariants,
                decreases,
            } => {
                // Optional label: 'loop_name:
                if let Some(lbl) = label {
                    stream.push(Token::new(
                        TokenKind::Lifetime(format!("'{}", lbl.as_str()).into()),
                        self.span,
                    ));
                    stream.push(Token::new(TokenKind::Colon, self.span));
                }

                // for await keywords
                stream.push(Token::new(TokenKind::For, self.span));
                stream.push(Token::new(TokenKind::Await, self.span));

                // pattern (e.g., x, (a, b), Item { field })
                pattern.to_tokens(stream);

                // in keyword
                stream.push(Token::new(TokenKind::In, self.span));

                // async iterable expression
                async_iterable.to_tokens(stream);

                // Loop invariant annotations
                for inv in invariants.iter() {
                    stream.push(Token::new(TokenKind::Invariant, self.span));
                    inv.to_tokens(stream);
                }

                // Decreases annotations for termination
                for dec in decreases.iter() {
                    stream.push(Token::new(TokenKind::Decreases, self.span));
                    dec.to_tokens(stream);
                }

                // Body block
                body.to_tokens(stream);
            }

            ExprKind::Break { label, value } => {
                stream.push(Token::new(TokenKind::Break, self.span));
                if let Some(label) = label {
                    stream.push(Token::new(
                        TokenKind::Ident(format!("'{}", label.as_str()).into()),
                        self.span,
                    ));
                }
                if let Some(val) = value {
                    val.to_tokens(stream);
                }
            }

            ExprKind::Continue { label } => {
                stream.push(Token::new(TokenKind::Continue, self.span));
                if let Some(label) = label {
                    stream.push(Token::new(
                        TokenKind::Ident(format!("'{}", label.as_str()).into()),
                        self.span,
                    ));
                }
            }

            ExprKind::Return(maybe_expr) => {
                stream.push(Token::new(TokenKind::Return, self.span));
                if let Some(expr) = maybe_expr {
                    expr.to_tokens(stream);
                }
            }

            ExprKind::Cast { expr, ty } => {
                expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::As, self.span));
                ty.to_tokens(stream);
            }

            ExprKind::Try(expr) => {
                expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::Question, self.span));
            }

            ExprKind::TryBlock(inner) => {
                // try { expr }
                stream.push(Token::new(TokenKind::Try, self.span));
                stream.push(Token::new(TokenKind::LBrace, self.span));
                inner.to_tokens(stream);
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            ExprKind::Record { path, fields, base } => {
                path.to_tokens(stream);
                stream.push(Token::new(TokenKind::LBrace, self.span));
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    stream.push(Token::new(
                        TokenKind::Ident(field.name.as_str().to_string().into()),
                        self.span,
                    ));
                    if let Some(ref value) = field.value {
                        stream.push(Token::new(TokenKind::Colon, self.span));
                        value.to_tokens(stream);
                    }
                }
                if let Some(base_expr) = base {
                    stream.push(Token::new(TokenKind::Comma, self.span));
                    stream.push(Token::new(TokenKind::DotDot, self.span));
                    base_expr.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            ExprKind::Closure {
                async_,
                move_,
                params,
                body,
                return_type,
                ..
            } => {
                if *async_ {
                    stream.push(Token::new(TokenKind::Async, self.span));
                }
                if *move_ {
                    stream.push(Token::new(TokenKind::Move, self.span));
                }
                stream.push(Token::new(TokenKind::Pipe, self.span));
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    param.pattern.to_tokens(stream);
                    if let verum_common::Maybe::Some(ref ty) = param.ty {
                        stream.push(Token::new(TokenKind::Colon, self.span));
                        ty.to_tokens(stream);
                    }
                }
                stream.push(Token::new(TokenKind::Pipe, self.span));
                if let verum_common::Maybe::Some(ret_ty) = return_type {
                    stream.push(Token::new(TokenKind::RArrow, self.span));
                    ret_ty.to_tokens(stream);
                }
                body.to_tokens(stream);
            }

            ExprKind::OptionalChain { expr, field } => {
                expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::Question, self.span));
                stream.push(Token::new(TokenKind::Dot, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(field.as_str().to_string().into()),
                    self.span,
                ));
            }

            ExprKind::TupleIndex { expr, index } => {
                expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::Dot, self.span));
                stream.push(Token::new(
                    TokenKind::Integer(IntegerLiteral {
                        raw_value: index.to_string().into(),
                        base: 10,
                        suffix: None,
                    }),
                    self.span,
                ));
            }

            ExprKind::Pipeline { left, right } => {
                left.to_tokens(stream);
                stream.push(Token::new(TokenKind::PipeGt, self.span));
                right.to_tokens(stream);
            }

            ExprKind::NullCoalesce { left, right } => {
                left.to_tokens(stream);
                stream.push(Token::new(TokenKind::QuestionQuestion, self.span));
                right.to_tokens(stream);
            }

            ExprKind::TryRecover {
                try_block,
                recover,
            } => {
                stream.push(Token::new(TokenKind::Try, self.span));
                try_block.to_tokens(stream);
                stream.push(Token::new(TokenKind::Recover, self.span));
                match recover {
                    RecoverBody::MatchArms { arms, .. } => {
                        stream.push(Token::new(TokenKind::LBrace, self.span));
                        for arm in arms.iter() {
                            arm.to_tokens(stream);
                        }
                        stream.push(Token::new(TokenKind::RBrace, self.span));
                    }
                    RecoverBody::Closure { param, body, .. } => {
                        stream.push(Token::new(TokenKind::Pipe, self.span));
                        param.pattern.to_tokens(stream);
                        stream.push(Token::new(TokenKind::Pipe, self.span));
                        body.to_tokens(stream);
                    }
                }
            }

            ExprKind::TryFinally {
                try_block,
                finally_block,
            } => {
                stream.push(Token::new(TokenKind::Try, self.span));
                try_block.to_tokens(stream);
                stream.push(Token::new(TokenKind::Finally, self.span));
                finally_block.to_tokens(stream);
            }

            ExprKind::TryRecoverFinally {
                try_block,
                recover,
                finally_block,
            } => {
                stream.push(Token::new(TokenKind::Try, self.span));
                try_block.to_tokens(stream);
                stream.push(Token::new(TokenKind::Recover, self.span));
                match recover {
                    RecoverBody::MatchArms { arms, .. } => {
                        stream.push(Token::new(TokenKind::LBrace, self.span));
                        for arm in arms.iter() {
                            arm.to_tokens(stream);
                        }
                        stream.push(Token::new(TokenKind::RBrace, self.span));
                    }
                    RecoverBody::Closure { param, body, .. } => {
                        stream.push(Token::new(TokenKind::Pipe, self.span));
                        param.pattern.to_tokens(stream);
                        stream.push(Token::new(TokenKind::Pipe, self.span));
                        body.to_tokens(stream);
                    }
                }
                stream.push(Token::new(TokenKind::Finally, self.span));
                finally_block.to_tokens(stream);
            }

            ExprKind::Array(array_expr) => {
                use verum_ast::expr::ArrayExpr;
                stream.push(Token::new(TokenKind::LBracket, self.span));
                match array_expr {
                    ArrayExpr::List(elements) => {
                        for (i, elem) in elements.iter().enumerate() {
                            if i > 0 {
                                stream.push(Token::new(TokenKind::Comma, self.span));
                            }
                            elem.to_tokens(stream);
                        }
                    }
                    ArrayExpr::Repeat { value, count } => {
                        value.to_tokens(stream);
                        stream.push(Token::new(TokenKind::Semicolon, self.span));
                        count.to_tokens(stream);
                    }
                }
                stream.push(Token::new(TokenKind::RBracket, self.span));
            }

            ExprKind::Comprehension { expr, clauses } => {
                use verum_ast::expr::ComprehensionClauseKind;
                stream.push(Token::new(TokenKind::LBracket, self.span));
                expr.to_tokens(stream);
                for clause in clauses.iter() {
                    match &clause.kind {
                        ComprehensionClauseKind::For { pattern, iter } => {
                            stream.push(Token::new(TokenKind::For, clause.span));
                            pattern.to_tokens(stream);
                            stream.push(Token::new(TokenKind::In, clause.span));
                            iter.to_tokens(stream);
                        }
                        ComprehensionClauseKind::If(cond) => {
                            stream.push(Token::new(TokenKind::If, clause.span));
                            cond.to_tokens(stream);
                        }
                        ComprehensionClauseKind::Let { pattern, ty, value } => {
                            stream.push(Token::new(TokenKind::Let, clause.span));
                            pattern.to_tokens(stream);
                            if let verum_common::Maybe::Some(t) = ty {
                                stream.push(Token::new(TokenKind::Colon, clause.span));
                                t.to_tokens(stream);
                            }
                            stream.push(Token::new(TokenKind::Eq, clause.span));
                            value.to_tokens(stream);
                        }
                    }
                }
                stream.push(Token::new(TokenKind::RBracket, self.span));
            }

            ExprKind::StreamComprehension { expr, clauses } => {
                use verum_ast::expr::ComprehensionClauseKind;
                stream.push(Token::new(TokenKind::Stream, self.span));
                stream.push(Token::new(TokenKind::LBracket, self.span));
                expr.to_tokens(stream);
                for clause in clauses.iter() {
                    match &clause.kind {
                        ComprehensionClauseKind::For { pattern, iter } => {
                            stream.push(Token::new(TokenKind::For, clause.span));
                            pattern.to_tokens(stream);
                            stream.push(Token::new(TokenKind::In, clause.span));
                            iter.to_tokens(stream);
                        }
                        ComprehensionClauseKind::If(cond) => {
                            stream.push(Token::new(TokenKind::If, clause.span));
                            cond.to_tokens(stream);
                        }
                        ComprehensionClauseKind::Let { pattern, ty, value } => {
                            stream.push(Token::new(TokenKind::Let, clause.span));
                            pattern.to_tokens(stream);
                            if let verum_common::Maybe::Some(t) = ty {
                                stream.push(Token::new(TokenKind::Colon, clause.span));
                                t.to_tokens(stream);
                            }
                            stream.push(Token::new(TokenKind::Eq, clause.span));
                            value.to_tokens(stream);
                        }
                    }
                }
                stream.push(Token::new(TokenKind::RBracket, self.span));
            }

            ExprKind::MapComprehension {
                key_expr,
                value_expr,
                clauses,
            } => {
                use verum_ast::expr::ComprehensionClauseKind;
                stream.push(Token::new(TokenKind::LBrace, self.span));
                key_expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::Colon, self.span));
                value_expr.to_tokens(stream);
                for clause in clauses.iter() {
                    match &clause.kind {
                        ComprehensionClauseKind::For { pattern, iter } => {
                            stream.push(Token::new(TokenKind::For, clause.span));
                            pattern.to_tokens(stream);
                            stream.push(Token::new(TokenKind::In, clause.span));
                            iter.to_tokens(stream);
                        }
                        ComprehensionClauseKind::If(cond) => {
                            stream.push(Token::new(TokenKind::If, clause.span));
                            cond.to_tokens(stream);
                        }
                        ComprehensionClauseKind::Let { pattern, ty, value } => {
                            stream.push(Token::new(TokenKind::Let, clause.span));
                            pattern.to_tokens(stream);
                            if let verum_common::Maybe::Some(t) = ty {
                                stream.push(Token::new(TokenKind::Colon, clause.span));
                                t.to_tokens(stream);
                            }
                            stream.push(Token::new(TokenKind::Eq, clause.span));
                            value.to_tokens(stream);
                        }
                    }
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            ExprKind::SetComprehension { expr, clauses } => {
                use verum_ast::expr::ComprehensionClauseKind;
                stream.push(Token::new(TokenKind::Set, self.span));
                stream.push(Token::new(TokenKind::LBrace, self.span));
                expr.to_tokens(stream);
                for clause in clauses.iter() {
                    match &clause.kind {
                        ComprehensionClauseKind::For { pattern, iter } => {
                            stream.push(Token::new(TokenKind::For, clause.span));
                            pattern.to_tokens(stream);
                            stream.push(Token::new(TokenKind::In, clause.span));
                            iter.to_tokens(stream);
                        }
                        ComprehensionClauseKind::If(cond) => {
                            stream.push(Token::new(TokenKind::If, clause.span));
                            cond.to_tokens(stream);
                        }
                        ComprehensionClauseKind::Let { pattern, ty, value } => {
                            stream.push(Token::new(TokenKind::Let, clause.span));
                            pattern.to_tokens(stream);
                            if let verum_common::Maybe::Some(t) = ty {
                                stream.push(Token::new(TokenKind::Colon, clause.span));
                                t.to_tokens(stream);
                            }
                            stream.push(Token::new(TokenKind::Eq, clause.span));
                            value.to_tokens(stream);
                        }
                    }
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            ExprKind::GeneratorComprehension { expr, clauses } => {
                use verum_ast::expr::ComprehensionClauseKind;
                stream.push(Token::new(TokenKind::Gen, self.span));
                stream.push(Token::new(TokenKind::LBrace, self.span));
                expr.to_tokens(stream);
                for clause in clauses.iter() {
                    match &clause.kind {
                        ComprehensionClauseKind::For { pattern, iter } => {
                            stream.push(Token::new(TokenKind::For, clause.span));
                            pattern.to_tokens(stream);
                            stream.push(Token::new(TokenKind::In, clause.span));
                            iter.to_tokens(stream);
                        }
                        ComprehensionClauseKind::If(cond) => {
                            stream.push(Token::new(TokenKind::If, clause.span));
                            cond.to_tokens(stream);
                        }
                        ComprehensionClauseKind::Let { pattern, ty, value } => {
                            stream.push(Token::new(TokenKind::Let, clause.span));
                            pattern.to_tokens(stream);
                            if let verum_common::Maybe::Some(t) = ty {
                                stream.push(Token::new(TokenKind::Colon, clause.span));
                                t.to_tokens(stream);
                            }
                            stream.push(Token::new(TokenKind::Eq, clause.span));
                            value.to_tokens(stream);
                        }
                    }
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            ExprKind::InterpolatedString {
                handler,
                parts,
                exprs,
            } => {
                // Output the handler prefix (e.g., "f", "sql")
                stream.push(Token::new(
                    TokenKind::Ident(handler.as_str().to_string().into()),
                    self.span,
                ));
                // Start the string
                stream.push(Token::new(TokenKind::Text("\"".into()), self.span));
                // Interleave parts and expressions
                for (i, part) in parts.iter().enumerate() {
                    stream.push(Token::new(
                        TokenKind::Text(part.as_str().to_string().into()),
                        self.span,
                    ));
                    if let Some(expr) = exprs.get(i) {
                        stream.push(Token::new(TokenKind::LBrace, self.span));
                        expr.to_tokens(stream);
                        stream.push(Token::new(TokenKind::RBrace, self.span));
                    }
                }
            }

            ExprKind::TensorLiteral {
                shape,
                elem_type,
                data,
            } => {
                stream.push(Token::new(
                    TokenKind::Ident("tensor".into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Lt, self.span));
                for (i, dim) in shape.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    stream.push(Token::new(
                        TokenKind::Integer(IntegerLiteral {
                            raw_value: dim.to_string().into(),
                            base: 10,
                            suffix: None,
                        }),
                        self.span,
                    ));
                }
                stream.push(Token::new(TokenKind::Gt, self.span));
                elem_type.to_tokens(stream);
                stream.push(Token::new(TokenKind::LBrace, self.span));
                data.to_tokens(stream);
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            ExprKind::MapLiteral { entries } => {
                stream.push(Token::new(TokenKind::LBrace, self.span));
                for (i, (key, value)) in entries.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    key.to_tokens(stream);
                    stream.push(Token::new(TokenKind::Colon, self.span));
                    value.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            ExprKind::SetLiteral { elements } => {
                stream.push(Token::new(TokenKind::LBrace, self.span));
                for (i, elem) in elements.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    elem.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            ExprKind::Yield(expr) => {
                stream.push(Token::new(TokenKind::Yield, self.span));
                expr.to_tokens(stream);
            }

            ExprKind::Typeof(expr) => {
                stream.push(Token::new(
                    TokenKind::Ident("typeof".into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::LParen, self.span));
                expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::RParen, self.span));
            }

            ExprKind::Async(block) => {
                stream.push(Token::new(TokenKind::Async, self.span));
                block.to_tokens(stream);
            }

            ExprKind::Await(expr) => {
                expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::Dot, self.span));
                stream.push(Token::new(TokenKind::Await, self.span));
            }

            ExprKind::Inject { type_path } => {
                stream.push(Token::new(TokenKind::Inject, self.span));
                // Type path is output as identifiers
                for seg in &type_path.segments {
                    if let verum_ast::ty::PathSegment::Name(ident) = seg {
                        stream.push(Token::new(TokenKind::Ident(ident.name.clone()), self.span));
                    }
                }
            }
            ExprKind::Spawn { expr, contexts } => {
                stream.push(Token::new(TokenKind::Spawn, self.span));
                expr.to_tokens(stream);
                if !contexts.is_empty() {
                    stream.push(Token::new(TokenKind::Using, self.span));
                    stream.push(Token::new(TokenKind::LBracket, self.span));
                    for (i, ctx) in contexts.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Comma, self.span));
                        }
                        // Inline ContextRequirement to tokens
                        ctx.path.to_tokens(stream);
                        if !ctx.args.is_empty() {
                            stream.push(Token::new(TokenKind::Lt, ctx.span));
                            for (j, arg) in ctx.args.iter().enumerate() {
                                if j > 0 {
                                    stream.push(Token::new(TokenKind::Comma, ctx.span));
                                }
                                arg.to_tokens(stream);
                            }
                            stream.push(Token::new(TokenKind::Gt, ctx.span));
                        }
                    }
                    stream.push(Token::new(TokenKind::RBracket, self.span));
                }
            }

            ExprKind::Unsafe(block) => {
                stream.push(Token::new(TokenKind::Unsafe, self.span));
                block.to_tokens(stream);
            }

            ExprKind::Meta(block) => {
                stream.push(Token::new(TokenKind::Meta, self.span));
                block.to_tokens(stream);
            }

            ExprKind::MacroCall { path, args } => {
                use verum_ast::expr::MacroDelimiter;
                path.to_tokens(stream);
                stream.push(Token::new(TokenKind::Bang, self.span));
                // Inline MacroArgs handling
                match args.delimiter {
                    MacroDelimiter::Paren => {
                        stream.push(Token::new(TokenKind::LParen, args.span));
                        stream.push(Token::new(
                            TokenKind::Ident(args.tokens.as_str().to_string().into()),
                            args.span,
                        ));
                        stream.push(Token::new(TokenKind::RParen, args.span));
                    }
                    MacroDelimiter::Bracket => {
                        stream.push(Token::new(TokenKind::LBracket, args.span));
                        stream.push(Token::new(
                            TokenKind::Ident(args.tokens.as_str().to_string().into()),
                            args.span,
                        ));
                        stream.push(Token::new(TokenKind::RBracket, args.span));
                    }
                    MacroDelimiter::Brace => {
                        stream.push(Token::new(TokenKind::LBrace, args.span));
                        stream.push(Token::new(
                            TokenKind::Ident(args.tokens.as_str().to_string().into()),
                            args.span,
                        ));
                        stream.push(Token::new(TokenKind::RBrace, args.span));
                    }
                }
            }

            ExprKind::UseContext {
                context,
                handler,
                body,
            } => {
                // Use Ident since TokenKind::Use doesn't exist
                stream.push(Token::new(TokenKind::Ident("use".into()), self.span));
                context.to_tokens(stream);
                stream.push(Token::new(TokenKind::Eq, self.span));
                handler.to_tokens(stream);
                stream.push(Token::new(TokenKind::In, self.span));
                body.to_tokens(stream);
            }

            ExprKind::Range {
                start,
                end,
                inclusive,
            } => {
                if let verum_common::Maybe::Some(s) = start {
                    s.to_tokens(stream);
                }
                if *inclusive {
                    stream.push(Token::new(TokenKind::DotDotEq, self.span));
                } else {
                    stream.push(Token::new(TokenKind::DotDot, self.span));
                }
                if let verum_common::Maybe::Some(e) = end {
                    e.to_tokens(stream);
                }
            }

            ExprKind::Forall { bindings, body } => {
                // Use Ident since TokenKind::Forall doesn't exist
                stream.push(Token::new(
                    TokenKind::Ident("forall".into()),
                    self.span,
                ));
                for (i, binding) in bindings.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    binding.pattern.to_tokens(stream);
                    if let verum_common::Maybe::Some(ty) = &binding.ty {
                        stream.push(Token::new(TokenKind::Colon, self.span));
                        ty.to_tokens(stream);
                    }
                    if let verum_common::Maybe::Some(domain) = &binding.domain {
                        stream.push(Token::new(TokenKind::Ident("in".into()), self.span));
                        domain.to_tokens(stream);
                    }
                    if let verum_common::Maybe::Some(guard) = &binding.guard {
                        stream.push(Token::new(TokenKind::Ident("where".into()), self.span));
                        guard.to_tokens(stream);
                    }
                }
                stream.push(Token::new(TokenKind::Dot, self.span));
                body.to_tokens(stream);
            }

            ExprKind::Exists { bindings, body } => {
                // Use Ident since TokenKind::Exists doesn't exist
                stream.push(Token::new(
                    TokenKind::Ident("exists".into()),
                    self.span,
                ));
                for (i, binding) in bindings.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    binding.pattern.to_tokens(stream);
                    if let verum_common::Maybe::Some(ty) = &binding.ty {
                        stream.push(Token::new(TokenKind::Colon, self.span));
                        ty.to_tokens(stream);
                    }
                    if let verum_common::Maybe::Some(domain) = &binding.domain {
                        stream.push(Token::new(TokenKind::Ident("in".into()), self.span));
                        domain.to_tokens(stream);
                    }
                    if let verum_common::Maybe::Some(guard) = &binding.guard {
                        stream.push(Token::new(TokenKind::Ident("where".into()), self.span));
                        guard.to_tokens(stream);
                    }
                }
                stream.push(Token::new(TokenKind::Dot, self.span));
                body.to_tokens(stream);
            }

            ExprKind::Paren(expr) => {
                stream.push(Token::new(TokenKind::LParen, self.span));
                expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::RParen, self.span));
            }

            ExprKind::Attenuate {
                context,
                capabilities,
            } => {
                context.to_tokens(stream);
                stream.push(Token::new(TokenKind::Dot, self.span));
                stream.push(Token::new(
                    TokenKind::Ident("attenuate".into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::LParen, self.span));
                // Output capabilities
                for (i, cap) in capabilities.capabilities.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Pipe, self.span));
                    }
                    stream.push(Token::new(
                        TokenKind::Ident(format!("{:?}", cap).into()),
                        self.span,
                    ));
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
            }

            ExprKind::TypeProperty { ty, property } => {
                // Output as Type.property (e.g., Int.size, Float.alignment)
                ty.to_tokens(stream);
                stream.push(Token::new(TokenKind::Dot, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(format!("{}", property).into()),
                    self.span,
                ));
            }

            ExprKind::Throw(inner) => {
                // Output as: throw expr
                stream.push(Token::new(
                    TokenKind::Ident("throw".into()),
                    self.span,
                ));
                inner.to_tokens(stream);
            }

            ExprKind::Select { arms, .. } => {
                // Output as: select { ... }
                stream.push(Token::new(
                    TokenKind::Ident("select".into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::LBrace, self.span));
                for arm in arms.iter() {
                    arm.body.to_tokens(stream);
                    stream.push(Token::new(TokenKind::Comma, self.span));
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            ExprKind::Is { expr, pattern, negated } => {
                // Output as: expr is pattern or expr is not pattern
                expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::Is, self.span));
                if *negated {
                    stream.push(Token::new(
                        TokenKind::Ident("not".into()),
                        self.span,
                    ));
                }
                pattern.to_tokens(stream);
            }

            ExprKind::TypeExpr(ty) => {
                // Output the type as tokens (for static method calls like List<Int>.new())
                ty.to_tokens(stream);
            }

            ExprKind::TypeBound { type_param, bound } => {
                // Output as: T: Protocol
                stream.push(Token::new(
                    TokenKind::Ident(type_param.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Colon, self.span));
                bound.to_tokens(stream);
            }

            ExprKind::MetaFunction { name, args } => {
                // Output as: @name or @name(args)
                stream.push(Token::new(TokenKind::At, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(name.as_str().to_string().into()),
                    self.span,
                ));
                if !args.is_empty() {
                    stream.push(Token::new(TokenKind::LParen, self.span));
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Comma, self.span));
                        }
                        arg.to_tokens(stream);
                    }
                    stream.push(Token::new(TokenKind::RParen, self.span));
                }
            }

            ExprKind::Nursery {
                options,
                body,
                on_cancel,
                recover,
                ..
            } => {
                // Output: nursery(options) { body } [on_cancel { ... }] [recover ...]
                stream.push(Token::new(TokenKind::Nursery, self.span));

                // Options
                if options.timeout.is_some() || options.max_tasks.is_some() {
                    stream.push(Token::new(TokenKind::LParen, self.span));
                    let mut first = true;
                    if let verum_common::Maybe::Some(timeout) = &options.timeout {
                        stream.push(Token::new(TokenKind::Ident("timeout".into()), self.span));
                        stream.push(Token::new(TokenKind::Colon, self.span));
                        timeout.to_tokens(stream);
                        first = false;
                    }
                    if let verum_common::Maybe::Some(max_tasks) = &options.max_tasks {
                        if !first {
                            stream.push(Token::new(TokenKind::Comma, self.span));
                        }
                        stream.push(Token::new(TokenKind::Ident("max_tasks".into()), self.span));
                        stream.push(Token::new(TokenKind::Colon, self.span));
                        max_tasks.to_tokens(stream);
                    }
                    stream.push(Token::new(TokenKind::RParen, self.span));
                }

                // Body block
                stream.push(Token::new(TokenKind::LBrace, self.span));
                for stmt in body.stmts.iter() {
                    stmt.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));

                // on_cancel block
                if let verum_common::Maybe::Some(cancel_block) = on_cancel {
                    stream.push(Token::new(TokenKind::Ident("on_cancel".into()), self.span));
                    stream.push(Token::new(TokenKind::LBrace, self.span));
                    for stmt in cancel_block.stmts.iter() {
                        stmt.to_tokens(stream);
                    }
                    stream.push(Token::new(TokenKind::RBrace, self.span));
                }

                // recover block
                if let verum_common::Maybe::Some(recover_body) = recover {
                    stream.push(Token::new(TokenKind::Recover, self.span));
                    match recover_body {
                        verum_ast::expr::RecoverBody::MatchArms { arms, .. } => {
                            stream.push(Token::new(TokenKind::LBrace, self.span));
                            for arm in arms.iter() {
                                arm.pattern.to_tokens(stream);
                                stream.push(Token::new(TokenKind::FatArrow, self.span));
                                arm.body.to_tokens(stream);
                                stream.push(Token::new(TokenKind::Comma, self.span));
                            }
                            stream.push(Token::new(TokenKind::RBrace, self.span));
                        }
                        verum_ast::expr::RecoverBody::Closure { param, body, .. } => {
                            stream.push(Token::new(TokenKind::Pipe, self.span));
                            param.pattern.to_tokens(stream);
                            stream.push(Token::new(TokenKind::Pipe, self.span));
                            body.to_tokens(stream);
                        }
                    }
                }
            }

            ExprKind::InlineAsm { template, operands, options } => {
                // Output: @asm("template", [operands], options(...))
                stream.push(Token::new(TokenKind::At, self.span));
                stream.push(Token::new(TokenKind::Ident("asm".into()), self.span));
                stream.push(Token::new(TokenKind::LParen, self.span));
                // Template string
                stream.push(Token::new(TokenKind::Text(template.to_string().into()), self.span));
                // Operands (if any)
                if !operands.is_empty() {
                    stream.push(Token::new(TokenKind::Comma, self.span));
                    stream.push(Token::new(TokenKind::LBracket, self.span));
                    for (i, operand) in operands.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Comma, self.span));
                        }
                        // Operand name if present
                        if let verum_common::Maybe::Some(name) = &operand.name {
                            stream.push(Token::new(TokenKind::Ident(name.name.to_string().into()), self.span));
                            stream.push(Token::new(TokenKind::Eq, self.span));
                        }
                        // Operand kind
                        use verum_ast::expr::AsmOperandKind;
                        match &operand.kind {
                            AsmOperandKind::In { constraint, expr } => {
                                stream.push(Token::new(TokenKind::Ident("in".into()), self.span));
                                stream.push(Token::new(TokenKind::LParen, self.span));
                                stream.push(Token::new(TokenKind::Text(constraint.constraint.to_string().into()), self.span));
                                stream.push(Token::new(TokenKind::RParen, self.span));
                                expr.to_tokens(stream);
                            }
                            AsmOperandKind::Out { constraint, place, late } => {
                                let kw = if *late { "lateout" } else { "out" };
                                stream.push(Token::new(TokenKind::Ident(kw.into()), self.span));
                                stream.push(Token::new(TokenKind::LParen, self.span));
                                stream.push(Token::new(TokenKind::Text(constraint.constraint.to_string().into()), self.span));
                                stream.push(Token::new(TokenKind::RParen, self.span));
                                place.to_tokens(stream);
                            }
                            AsmOperandKind::InOut { constraint, place } => {
                                stream.push(Token::new(TokenKind::Ident("inout".into()), self.span));
                                stream.push(Token::new(TokenKind::LParen, self.span));
                                stream.push(Token::new(TokenKind::Text(constraint.constraint.to_string().into()), self.span));
                                stream.push(Token::new(TokenKind::RParen, self.span));
                                place.to_tokens(stream);
                            }
                            AsmOperandKind::InLateOut { constraint, in_expr, out_place } => {
                                stream.push(Token::new(TokenKind::Ident("inlateout".into()), self.span));
                                stream.push(Token::new(TokenKind::LParen, self.span));
                                stream.push(Token::new(TokenKind::Text(constraint.constraint.to_string().into()), self.span));
                                stream.push(Token::new(TokenKind::RParen, self.span));
                                in_expr.to_tokens(stream);
                                stream.push(Token::new(TokenKind::FatArrow, self.span));
                                out_place.to_tokens(stream);
                            }
                            AsmOperandKind::Sym { path } => {
                                stream.push(Token::new(TokenKind::Ident("sym".into()), self.span));
                                path.to_tokens(stream);
                            }
                            AsmOperandKind::Const { expr } => {
                                stream.push(Token::new(TokenKind::Const, self.span));
                                expr.to_tokens(stream);
                            }
                            AsmOperandKind::Clobber { reg } => {
                                stream.push(Token::new(TokenKind::Ident("clobber_abi".into()), self.span));
                                stream.push(Token::new(TokenKind::LParen, self.span));
                                stream.push(Token::new(TokenKind::Text(reg.to_string().into()), self.span));
                                stream.push(Token::new(TokenKind::RParen, self.span));
                            }
                        }
                    }
                    stream.push(Token::new(TokenKind::RBracket, self.span));
                }
                // Options
                let has_options = options.volatile || options.intel_syntax || !options.raw_options.is_empty();
                if has_options {
                    stream.push(Token::new(TokenKind::Comma, self.span));
                    stream.push(Token::new(TokenKind::Ident("options".into()), self.span));
                    stream.push(Token::new(TokenKind::LParen, self.span));
                    let mut first = true;
                    if options.volatile {
                        stream.push(Token::new(TokenKind::Ident("volatile".into()), self.span));
                        first = false;
                    }
                    if options.intel_syntax {
                        if !first {
                            stream.push(Token::new(TokenKind::Comma, self.span));
                        }
                        stream.push(Token::new(TokenKind::Ident("intel".into()), self.span));
                    }
                    stream.push(Token::new(TokenKind::RParen, self.span));
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
            }

            ExprKind::StreamLiteral(stream_lit) => {
                // Output: stream[elements] or stream[start..end]
                use verum_ast::expr::StreamLiteralKind;

                stream.push(Token::new(TokenKind::Stream, self.span));
                stream.push(Token::new(TokenKind::LBracket, self.span));

                match &stream_lit.kind {
                    StreamLiteralKind::Elements { elements, cycles } => {
                        for (i, elem) in elements.iter().enumerate() {
                            if i > 0 {
                                stream.push(Token::new(TokenKind::Comma, self.span));
                            }
                            elem.to_tokens(stream);
                        }
                        if *cycles {
                            // Add ... for cycling streams
                            if !elements.is_empty() {
                                stream.push(Token::new(TokenKind::Comma, self.span));
                            }
                            stream.push(Token::new(TokenKind::DotDotDot, self.span));
                        }
                    }
                    StreamLiteralKind::Range { start, end, inclusive } => {
                        start.to_tokens(stream);
                        if *inclusive {
                            stream.push(Token::new(TokenKind::DotDotEq, self.span));
                        } else {
                            stream.push(Token::new(TokenKind::DotDot, self.span));
                        }
                        if let verum_common::Maybe::Some(end_expr) = end {
                            end_expr.to_tokens(stream);
                        }
                    }
                }

                stream.push(Token::new(TokenKind::RBracket, self.span));
            }

            ExprKind::Quote { target_stage, tokens } => {
                // Output: quote { tokens } or quote(N) { tokens }
                stream.push(Token::new(TokenKind::QuoteKeyword, self.span));
                if let Some(stage) = target_stage {
                    stream.push(Token::new(TokenKind::At, self.span));
                    stream.push(Token::new(TokenKind::LParen, self.span));
                    stream.push(Token::new(
                        TokenKind::Integer(IntegerLiteral {
                            raw_value: stage.to_string().into(),
                            base: 10,
                            suffix: None,
                        }),
                        self.span,
                    ));
                    stream.push(Token::new(TokenKind::RParen, self.span));
                }
                stream.push(Token::new(TokenKind::LBrace, self.span));
                // Emit the token tree contents
                for token_tree in tokens.iter() {
                    token_tree.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            ExprKind::StageEscape { stage, expr } => {
                // Output: $(stage N){ expr }
                stream.push(Token::new(TokenKind::Dollar, self.span));
                stream.push(Token::new(TokenKind::LParen, self.span));
                stream.push(Token::new(TokenKind::Stage, self.span));
                stream.push(Token::new(
                    TokenKind::Integer(IntegerLiteral {
                        raw_value: stage.to_string().into(),
                        base: 10,
                        suffix: None,
                    }),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::RParen, self.span));
                stream.push(Token::new(TokenKind::LBrace, self.span));
                expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            ExprKind::Lift { expr } => {
                // Output: lift(expr)
                stream.push(Token::new(TokenKind::Lift, self.span));
                stream.push(Token::new(TokenKind::LParen, self.span));
                expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::RParen, self.span));
            }

            ExprKind::DestructuringAssign { pattern, op, value } => {
                // Output: pattern op value
                pattern.to_tokens(stream);
                // Convert BinOp to TokenKind for assignment operators
                let op_kind = match op {
                    BinOp::Assign => TokenKind::Eq,
                    BinOp::AddAssign => TokenKind::PlusEq,
                    BinOp::SubAssign => TokenKind::MinusEq,
                    BinOp::MulAssign => TokenKind::StarEq,
                    BinOp::DivAssign => TokenKind::SlashEq,
                    BinOp::RemAssign => TokenKind::PercentEq,
                    BinOp::BitAndAssign => TokenKind::AmpersandEq,
                    BinOp::BitOrAssign => TokenKind::PipeEq,
                    BinOp::BitXorAssign => TokenKind::CaretEq,
                    BinOp::ShlAssign => TokenKind::LtLtEq,
                    BinOp::ShrAssign => TokenKind::GtGtEq,
                    _ => TokenKind::Eq, // Fallback for unexpected operators
                };
                stream.push(Token::new(op_kind, self.span));
                value.to_tokens(stream);
            }
            ExprKind::CalcBlock(_) => {
                // Calc blocks in token output: emit as a comment/noop
            }
            ExprKind::NamedArg { name, value } => {
                // Emit as name: value
                stream.push(Token::new(
                    TokenKind::Ident(name.name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Colon, self.span));
                value.to_tokens(stream);
            }
            ExprKind::CopatternBody { arms, .. } => {
                // Emit copattern body: { .obs1 => expr1, .obs2 => expr2, ... }
                stream.push(Token::new(TokenKind::LBrace, self.span));
                let mut first = true;
                for arm in arms.iter() {
                    if !first {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    first = false;
                    stream.push(Token::new(TokenKind::Dot, self.span));
                    stream.push(Token::new(
                        TokenKind::Ident(arm.observation.name.as_str().to_string().into()),
                        self.span,
                    ));
                    stream.push(Token::new(TokenKind::FatArrow, self.span));
                    arm.body.as_ref().to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }
        }
    }
}

impl ToTokens for verum_ast::expr::TokenTree {
    fn to_tokens(&self, stream: &mut TokenStream) {
        use verum_ast::expr::{MacroDelimiter, TokenTree, TokenTreeKind};

        match self {
            TokenTree::Token(tok) => {
                // Convert TokenTreeToken back to lexer Token
                let kind = match &tok.kind {
                    TokenTreeKind::Ident => {
                        TokenKind::Ident(tok.text.as_str().to_string().into())
                    }
                    TokenTreeKind::IntLiteral => {
                        TokenKind::Integer(IntegerLiteral {
                            raw_value: tok.text.as_str().to_string().into(),
                            base: 10,
                            suffix: None,
                        })
                    }
                    TokenTreeKind::FloatLiteral => {
                        // Parse float value from text
                        let value = tok.text.as_str().parse().unwrap_or(0.0);
                        TokenKind::Float(FloatLiteral { value, suffix: None, raw: tok.text.clone() })
                    }
                    TokenTreeKind::StringLiteral => {
                        TokenKind::Text(tok.text.as_str().to_string().into())
                    }
                    TokenTreeKind::CharLiteral => {
                        let c = tok.text.as_str().chars().next().unwrap_or('\0');
                        TokenKind::Char(c)
                    }
                    TokenTreeKind::BoolLiteral => {
                        if tok.text.as_str() == "true" {
                            TokenKind::True
                        } else {
                            TokenKind::False
                        }
                    }
                    TokenTreeKind::Punct => {
                        // Map punctuation to token kind
                        match tok.text.as_str() {
                            "+" => TokenKind::Plus,
                            "-" => TokenKind::Minus,
                            "*" => TokenKind::Star,
                            "/" => TokenKind::Slash,
                            "%" => TokenKind::Percent,
                            "=" => TokenKind::Eq,
                            "==" => TokenKind::EqEq,
                            "!=" => TokenKind::BangEq,
                            "<" => TokenKind::Lt,
                            ">" => TokenKind::Gt,
                            "<=" => TokenKind::LtEq,
                            ">=" => TokenKind::GtEq,
                            "&&" => TokenKind::AmpersandAmpersand,
                            "||" => TokenKind::PipePipe,
                            "!" => TokenKind::Bang,
                            "&" => TokenKind::Ampersand,
                            "|" => TokenKind::Pipe,
                            "^" => TokenKind::Caret,
                            "~" => TokenKind::Tilde,
                            "." => TokenKind::Dot,
                            "," => TokenKind::Comma,
                            ";" => TokenKind::Semicolon,
                            ":" => TokenKind::Colon,
                            "::" => TokenKind::ColonColon,
                            "->" => TokenKind::RArrow,
                            "=>" => TokenKind::FatArrow,
                            "@" => TokenKind::At,
                            "?" => TokenKind::Question,
                            ".." => TokenKind::DotDot,
                            "..=" => TokenKind::DotDotEq,
                            "..." => TokenKind::DotDotDot,
                            _ => TokenKind::Ident(tok.text.as_str().to_string().into()),
                        }
                    }
                    TokenTreeKind::Keyword => {
                        // Map keywords to token kinds
                        match tok.text.as_str() {
                            "fn" => TokenKind::Fn,
                            "let" => TokenKind::Let,
                            "if" => TokenKind::If,
                            "else" => TokenKind::Else,
                            "match" => TokenKind::Match,
                            "while" => TokenKind::While,
                            "for" => TokenKind::For,
                            "loop" => TokenKind::Loop,
                            "return" => TokenKind::Return,
                            "break" => TokenKind::Break,
                            "continue" => TokenKind::Continue,
                            "type" => TokenKind::Type,
                            "is" => TokenKind::Is,
                            "in" => TokenKind::In,
                            "as" => TokenKind::As,
                            "mut" => TokenKind::Mut,
                            "public" => TokenKind::Public,
                            "module" => TokenKind::Module,
                            "mount" => TokenKind::Mount,
                            "async" => TokenKind::Async,
                            "await" => TokenKind::Await,
                            "meta" => TokenKind::Meta,
                            "quote" => TokenKind::QuoteKeyword,
                            _ => TokenKind::Ident(tok.text.as_str().to_string().into()),
                        }
                    }
                    TokenTreeKind::Eof => return, // Don't emit EOF tokens
                };
                stream.push(Token::new(kind, tok.span));
            }
            TokenTree::Group { delimiter, tokens, span } => {
                // Emit opening delimiter
                let (open, close) = match delimiter {
                    MacroDelimiter::Paren => (TokenKind::LParen, TokenKind::RParen),
                    MacroDelimiter::Bracket => (TokenKind::LBracket, TokenKind::RBracket),
                    MacroDelimiter::Brace => (TokenKind::LBrace, TokenKind::RBrace),
                };
                stream.push(Token::new(open, *span));
                // Emit nested tokens
                for token in tokens.iter() {
                    token.to_tokens(stream);
                }
                // Emit closing delimiter
                stream.push(Token::new(close, *span));
            }
        }
    }
}

impl ToTokens for Type {
    fn to_tokens(&self, stream: &mut TokenStream) {
        use verum_ast::ty::TypeKind;

        match &self.kind {
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    stream.push(Token::new(
                        TokenKind::Ident(ident.as_str().to_string().into()),
                        self.span,
                    ));
                } else {
                    for (i, segment) in path.segments.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Dot, self.span));
                        }
                        if let verum_ast::ty::PathSegment::Name(ident) = segment {
                            stream.push(Token::new(
                                TokenKind::Ident(ident.as_str().to_string().into()),
                                self.span,
                            ));
                        }
                    }
                }
            }

            TypeKind::Function {
                params,
                return_type,
                ..
            } => {
                stream.push(Token::new(TokenKind::Fn, self.span));
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    param.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
                stream.push(Token::new(TokenKind::RArrow, self.span));
                return_type.to_tokens(stream);
            }

            TypeKind::Rank2Function {
                type_params,
                params,
                return_type,
                ..
            } => {
                // Rank-2 function type: fn<R>(Reducer<B, R>) -> Reducer<A, R>
                stream.push(Token::new(TokenKind::Fn, self.span));
                // Output type parameters: <R, S, ...>
                if !type_params.is_empty() {
                    stream.push(Token::new(TokenKind::Lt, self.span));
                    for (i, param) in type_params.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Comma, self.span));
                        }
                        param.to_tokens(stream);
                    }
                    stream.push(Token::new(TokenKind::Gt, self.span));
                }
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    param.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
                stream.push(Token::new(TokenKind::RArrow, self.span));
                return_type.to_tokens(stream);
            }

            TypeKind::Tuple(types) => {
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    ty.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
            }

            TypeKind::Reference { mutable, inner } => {
                stream.push(Token::new(TokenKind::Ampersand, self.span));
                if *mutable {
                    stream.push(Token::new(TokenKind::Ident("mut".into()), self.span));
                }
                inner.to_tokens(stream);
            }

            TypeKind::Array { element, size } => {
                stream.push(Token::new(TokenKind::LBracket, self.span));
                element.to_tokens(stream);
                if let Some(size_expr) = size {
                    stream.push(Token::new(TokenKind::Semicolon, self.span));
                    size_expr.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RBracket, self.span));
            }

            TypeKind::Slice(element) => {
                stream.push(Token::new(TokenKind::LBracket, self.span));
                element.to_tokens(stream);
                stream.push(Token::new(TokenKind::RBracket, self.span));
            }

            TypeKind::Inferred => {
                stream.push(Token::new(TokenKind::Ident("_".into()), self.span));
            }

            TypeKind::Unit => {
                stream.push(Token::new(TokenKind::LParen, self.span));
                stream.push(Token::new(TokenKind::RParen, self.span));
            }

            TypeKind::Never => {
                // Never type is represented as ! in Verum syntax
                stream.push(Token::new(TokenKind::Bang, self.span));
            }

            TypeKind::Bool => {
                stream.push(Token::new(TokenKind::Ident("Bool".into()), self.span));
            }

            TypeKind::Int => {
                stream.push(Token::new(TokenKind::Ident("Int".into()), self.span));
            }

            TypeKind::Float => {
                stream.push(Token::new(TokenKind::Ident("Float".into()), self.span));
            }

            TypeKind::Char => {
                stream.push(Token::new(TokenKind::Ident("Char".into()), self.span));
            }

            TypeKind::Text => {
                stream.push(Token::new(TokenKind::Ident("Text".into()), self.span));
            }

            TypeKind::Generic { base, args } => {
                base.to_tokens(stream);
                stream.push(Token::new(TokenKind::Lt, self.span));
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    arg.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::Gt, self.span));
            }

            TypeKind::CheckedReference { mutable, inner } => {
                stream.push(Token::new(TokenKind::Ampersand, self.span));
                stream.push(Token::new(
                    TokenKind::Ident("checked".into()),
                    self.span,
                ));
                if *mutable {
                    stream.push(Token::new(TokenKind::Ident("mut".into()), self.span));
                }
                inner.to_tokens(stream);
            }

            TypeKind::UnsafeReference { mutable, inner } => {
                stream.push(Token::new(TokenKind::Ampersand, self.span));
                stream.push(Token::new(
                    TokenKind::Ident("unsafe".into()),
                    self.span,
                ));
                if *mutable {
                    stream.push(Token::new(TokenKind::Ident("mut".into()), self.span));
                }
                inner.to_tokens(stream);
            }

            TypeKind::Pointer { mutable, inner } => {
                stream.push(Token::new(TokenKind::Star, self.span));
                if *mutable {
                    stream.push(Token::new(TokenKind::Ident("mut".into()), self.span));
                } else {
                    stream.push(Token::new(TokenKind::Ident("const".into()), self.span));
                }
                inner.to_tokens(stream);
            }

            TypeKind::VolatilePointer { mutable, inner } => {
                stream.push(Token::new(TokenKind::Star, self.span));
                stream.push(Token::new(TokenKind::Ident("volatile".into()), self.span));
                if *mutable {
                    stream.push(Token::new(TokenKind::Ident("mut".into()), self.span));
                }
                inner.to_tokens(stream);
            }

            TypeKind::Qualified {
                self_ty,
                trait_ref,
                assoc_name,
            } => {
                stream.push(Token::new(TokenKind::Lt, self.span));
                self_ty.to_tokens(stream);
                stream.push(Token::new(TokenKind::Ident("as".into()), self.span));
                trait_ref.to_tokens(stream);
                stream.push(Token::new(TokenKind::Gt, self.span));
                stream.push(Token::new(TokenKind::Colon, self.span));
                stream.push(Token::new(TokenKind::Colon, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(assoc_name.as_str().to_string().into()),
                    self.span,
                ));
            }

            TypeKind::Refined { base, predicate } => {
                base.to_tokens(stream);
                stream.push(Token::new(TokenKind::LBrace, self.span));
                predicate.expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            TypeKind::Sigma {
                name,
                base,
                predicate,
            } => {
                stream.push(Token::new(
                    TokenKind::Ident(name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Colon, self.span));
                base.to_tokens(stream);
                stream.push(Token::new(TokenKind::Ident("where".into()), self.span));
                predicate.to_tokens(stream);
            }

            TypeKind::Bounded { base, bounds } => {
                base.to_tokens(stream);
                if !bounds.is_empty() {
                    stream.push(Token::new(TokenKind::Ident("where".into()), self.span));
                    for (i, bound) in bounds.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Plus, self.span));
                        }
                        bound.to_tokens(stream);
                    }
                }
            }

            TypeKind::DynProtocol { bounds, bindings } => {
                stream.push(Token::new(TokenKind::Ident("dyn".into()), self.span));
                for (i, bound) in bounds.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Plus, self.span));
                    }
                    bound.to_tokens(stream);
                }
                if let Some(binds) = bindings {
                    stream.push(Token::new(TokenKind::Lt, self.span));
                    for (i, binding) in binds.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Comma, self.span));
                        }
                        stream.push(Token::new(
                            TokenKind::Ident(binding.name.as_str().to_string().into()),
                            self.span,
                        ));
                        stream.push(Token::new(TokenKind::Eq, self.span));
                        binding.ty.to_tokens(stream);
                    }
                    stream.push(Token::new(TokenKind::Gt, self.span));
                }
            }

            TypeKind::Ownership { mutable, inner } => {
                stream.push(Token::new(TokenKind::Percent, self.span));
                if *mutable {
                    stream.push(Token::new(TokenKind::Ident("mut".into()), self.span));
                }
                inner.to_tokens(stream);
            }

            TypeKind::GenRef { inner } => {
                stream.push(Token::new(
                    TokenKind::Ident("GenRef".into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Lt, self.span));
                inner.to_tokens(stream);
                stream.push(Token::new(TokenKind::Gt, self.span));
            }

            TypeKind::TypeConstructor { base, arity } => {
                base.to_tokens(stream);
                stream.push(Token::new(TokenKind::Lt, self.span));
                for i in 0..*arity {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    stream.push(Token::new(TokenKind::Ident("_".into()), self.span));
                }
                stream.push(Token::new(TokenKind::Gt, self.span));
            }

            TypeKind::Tensor {
                element,
                shape,
                layout: _,
            } => {
                stream.push(Token::new(
                    TokenKind::Ident("Tensor".into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Lt, self.span));
                element.to_tokens(stream);
                stream.push(Token::new(TokenKind::Comma, self.span));
                stream.push(Token::new(TokenKind::LBracket, self.span));
                for (i, dim) in shape.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    dim.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RBracket, self.span));
                stream.push(Token::new(TokenKind::Gt, self.span));
            }

            TypeKind::Existential { name, bounds } => {
                stream.push(Token::new(TokenKind::Ident("some".into()), self.span));
                stream.push(Token::new(
                    TokenKind::Ident(name.as_str().to_string().into()),
                    self.span,
                ));
                if !bounds.is_empty() {
                    stream.push(Token::new(TokenKind::Colon, self.span));
                    for (i, bound) in bounds.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Plus, self.span));
                        }
                        bound.to_tokens(stream);
                    }
                }
            }

            TypeKind::AssociatedType { base, assoc } => {
                base.to_tokens(stream);
                stream.push(Token::new(TokenKind::Dot, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(assoc.as_str().to_string().into()),
                    self.span,
                ));
            }
            TypeKind::CapabilityRestricted { base, capabilities } => {
                // Format as: BaseType with [Cap1, Cap2, ...]
                base.to_tokens(stream);
                stream.push(Token::new(TokenKind::Ident("with".into()), self.span));
                stream.push(Token::new(TokenKind::LBracket, self.span));
                for (i, cap) in capabilities.capabilities.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    stream.push(Token::new(
                        TokenKind::Ident(cap.as_str().to_string().into()),
                        self.span,
                    ));
                }
                stream.push(Token::new(TokenKind::RBracket, self.span));
            }

            // Path type: Path<carrier>(lhs, rhs) — propositional equality
            TypeKind::PathType { carrier, lhs, rhs } => {
                stream.push(Token::new(TokenKind::Ident("Path".into()), self.span));
                stream.push(Token::new(TokenKind::Lt, self.span));
                carrier.to_tokens(stream);
                stream.push(Token::new(TokenKind::Gt, self.span));
                stream.push(Token::new(TokenKind::LParen, self.span));
                lhs.to_tokens(stream);
                stream.push(Token::new(TokenKind::Comma, self.span));
                rhs.to_tokens(stream);
                stream.push(Token::new(TokenKind::RParen, self.span));
            }

            // General dependent-type application: `carrier(v1, v2, ..)`
            // where `carrier` already embeds its `<type_args>` (if any).
            TypeKind::DependentApp { carrier, value_args } => {
                carrier.to_tokens(stream);
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, arg) in value_args.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    arg.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
            }

            // Unknown type - a safe top type
            TypeKind::Unknown => {
                stream.push(Token::new(TokenKind::Ident("Unknown".into()), self.span));
            }

            // Record type: { field1: Type1, field2: Type2, ... }
            TypeKind::Record { fields, .. } => {
                stream.push(Token::new(TokenKind::LBrace, self.span));
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    stream.push(Token::new(
                        TokenKind::Ident(field.name.as_str().to_string().into()),
                        field.name.span,
                    ));
                    stream.push(Token::new(TokenKind::Colon, field.name.span));
                    field.ty.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            // Universe type: Type or Type(n)
            TypeKind::Universe { .. } => {
                stream.push(Token::new(TokenKind::Ident("Type".into()), self.span));
            }

            TypeKind::Meta { inner } => {
                stream.push(Token::new(TokenKind::Meta, self.span));
                inner.to_tokens(stream);
            }

            TypeKind::TypeLambda { params, body } => {
                stream.push(Token::new(TokenKind::Pipe, self.span));
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    stream.push(Token::new(TokenKind::Ident(param.name.clone()), self.span));
                }
                stream.push(Token::new(TokenKind::Pipe, self.span));
                body.to_tokens(stream);
            }
        }
    }
}

impl ToTokens for verum_ast::ty::GenericArg {
    fn to_tokens(&self, stream: &mut TokenStream) {
        match self {
            verum_ast::ty::GenericArg::Type(ty) => ty.to_tokens(stream),
            verum_ast::ty::GenericArg::Const(expr) => expr.to_tokens(stream),
            verum_ast::ty::GenericArg::Lifetime(lt) => {
                // Lifetime: 'a - use Lifetime token directly
                stream.push(Token::new(TokenKind::Lifetime(lt.name.clone()), lt.span));
            }
            verum_ast::ty::GenericArg::Binding(binding) => {
                // Type binding: Target = SomeType
                stream.push(Token::new(
                    TokenKind::Ident(binding.name.as_str().to_string().into()),
                    binding.span,
                ));
                stream.push(Token::new(TokenKind::Eq, binding.span));
                binding.ty.to_tokens(stream);
            }
        }
    }
}

impl ToTokens for verum_ast::ty::TypeBound {
    fn to_tokens(&self, stream: &mut TokenStream) {
        use verum_ast::ty::TypeBoundKind;
        match &self.kind {
            TypeBoundKind::Protocol(path) => {
                // Protocol bound: Display or Numeric<T>
                path.to_tokens(stream);
            }
            TypeBoundKind::Equality(ty) => {
                // Equality bound: T = ConcreteType
                stream.push(Token::new(TokenKind::Eq, self.span));
                ty.to_tokens(stream);
            }
            TypeBoundKind::NegativeProtocol(path) => {
                // Negative protocol bound: T: !Trait
                stream.push(Token::new(TokenKind::Bang, self.span));
                path.to_tokens(stream);
            }
            TypeBoundKind::AssociatedTypeBound {
                type_path,
                assoc_name,
                bounds,
            } => {
                // Associated type bound: T.Item: Display
                type_path.to_tokens(stream);
                stream.push(Token::new(TokenKind::Dot, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(assoc_name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Colon, self.span));
                for (i, bound) in bounds.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Plus, self.span));
                    }
                    bound.to_tokens(stream);
                }
            }
            TypeBoundKind::AssociatedTypeEquality {
                type_path,
                assoc_name,
                eq_type,
            } => {
                // Associated type equality: T.Item = String
                type_path.to_tokens(stream);
                stream.push(Token::new(TokenKind::Dot, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(assoc_name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Eq, self.span));
                eq_type.to_tokens(stream);
            }
            TypeBoundKind::GenericProtocol(ty) => {
                // Generic protocol bound: Iterator<Item = T>
                ty.to_tokens(stream);
            }
        }
    }
}

impl ToTokens for verum_ast::ty::GenericParam {
    fn to_tokens(&self, stream: &mut TokenStream) {
        use verum_ast::ty::GenericParamKind;

        // Implicit parameters are wrapped in braces: {T} instead of T
        // Dependent type quotation: meta-level dependent type expressions.
        if self.is_implicit {
            stream.push(Token::new(TokenKind::LBrace, self.span));
        }

        match &self.kind {
            GenericParamKind::Type { name, bounds, default } => {
                // Type parameter: T or T: Clone or T: Clone = DefaultType
                stream.push(Token::new(
                    TokenKind::Ident(name.as_str().to_string().into()),
                    self.span,
                ));
                if !bounds.is_empty() {
                    stream.push(Token::new(TokenKind::Colon, self.span));
                    for (i, bound) in bounds.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Plus, self.span));
                        }
                        bound.to_tokens(stream);
                    }
                }
                if let verum_common::Maybe::Some(default_ty) = default {
                    stream.push(Token::new(TokenKind::Eq, self.span));
                    default_ty.to_tokens(stream);
                }
            }
            GenericParamKind::HigherKinded { name, arity, bounds } => {
                // Higher-kinded: F<_> or F<_, _>: Functor
                stream.push(Token::new(
                    TokenKind::Ident(name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Lt, self.span));
                for i in 0..*arity {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    stream.push(Token::new(TokenKind::Ident("_".into()), self.span));
                }
                stream.push(Token::new(TokenKind::Gt, self.span));
                if !bounds.is_empty() {
                    stream.push(Token::new(TokenKind::Colon, self.span));
                    for (i, bound) in bounds.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Plus, self.span));
                        }
                        bound.to_tokens(stream);
                    }
                }
            }
            GenericParamKind::Const { name, ty } => {
                // Const parameter: const N: usize
                stream.push(Token::new(TokenKind::Const, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Colon, self.span));
                ty.to_tokens(stream);
            }
            GenericParamKind::Meta { name, ty, refinement } => {
                // Meta parameter: N: meta usize
                stream.push(Token::new(
                    TokenKind::Ident(name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Colon, self.span));
                stream.push(Token::new(TokenKind::Meta, self.span));
                ty.to_tokens(stream);
                // Only add refinement braces if NOT implicit (for implicit, outer braces already there)
                if !self.is_implicit {
                    if let verum_common::Maybe::Some(ref_pred) = refinement {
                        stream.push(Token::new(TokenKind::LBrace, self.span));
                        ref_pred.to_tokens(stream);
                        stream.push(Token::new(TokenKind::RBrace, self.span));
                    }
                }
            }
            GenericParamKind::Lifetime { name } => {
                // Lifetime parameter: 'a
                stream.push(Token::new(TokenKind::Lifetime(name.name.clone()), self.span));
            }
            GenericParamKind::Context { name } => {
                // Context parameter: using C
                stream.push(Token::new(TokenKind::Using, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(name.as_str().to_string().into()),
                    self.span,
                ));
            }
            GenericParamKind::Level { name, .. } => {
                // Universe level parameter: u: Level — skip in token output
                stream.push(Token::new(
                    TokenKind::Ident(name.as_str().to_string().into()),
                    self.span,
                ));
            }
            GenericParamKind::KindAnnotated { name, bounds, .. } => {
                // Kind-annotated type constructor: F: Type -> Type
                stream.push(Token::new(
                    TokenKind::Ident(name.as_str().to_string().into()),
                    self.span,
                ));
                // Emit bounds if present
                if !bounds.is_empty() {
                    stream.push(Token::new(TokenKind::Colon, self.span));
                    for (i, bound) in bounds.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Plus, self.span));
                        }
                        bound.to_tokens(stream);
                    }
                }
            }
        }

        // Close brace for implicit parameters
        if self.is_implicit {
            stream.push(Token::new(TokenKind::RBrace, self.span));
        }
    }
}

impl ToTokens for Pattern {
    fn to_tokens(&self, stream: &mut TokenStream) {
        use verum_ast::pattern::PatternKind;

        match &self.kind {
            PatternKind::Wildcard => {
                stream.push(Token::new(TokenKind::Ident("_".into()), self.span));
            }

            PatternKind::Ident {
                mutable,
                name,
                by_ref,
                subpattern,
            } => {
                if *by_ref {
                    stream.push(Token::new(TokenKind::Ident("ref".into()), self.span));
                }
                if *mutable {
                    stream.push(Token::new(TokenKind::Ident("mut".into()), self.span));
                }
                stream.push(Token::new(
                    TokenKind::Ident(name.as_str().to_string().into()),
                    self.span,
                ));
                if let Some(subpat) = subpattern {
                    stream.push(Token::new(TokenKind::At, self.span));
                    subpat.to_tokens(stream);
                }
            }

            PatternKind::Literal(lit) => {
                use verum_ast::LiteralKind;
                match &lit.kind {
                    LiteralKind::Bool(b) => {
                        let kind = if *b {
                            TokenKind::True
                        } else {
                            TokenKind::False
                        };
                        stream.push(Token::new(kind, self.span));
                    }
                    LiteralKind::Int(i) => {
                        stream.push(Token::new(
                            TokenKind::Integer(IntegerLiteral {
                                raw_value: i.value.to_string().into(),
                                base: 10,
                                suffix: None,
                            }),
                            self.span,
                        ));
                    }
                    LiteralKind::Text(s) => {
                        stream.push(Token::new(
                            TokenKind::Text(s.as_str().to_string().into()),
                            self.span,
                        ));
                    }
                    LiteralKind::Char(c) => {
                        stream.push(Token::new(TokenKind::Char(*c), self.span));
                    }
                    _ => {
                        stream.push(Token::new(
                            TokenKind::Ident("LITERAL_PATTERN".into()),
                            self.span,
                        ));
                    }
                }
            }

            PatternKind::Tuple(patterns) => {
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, pat) in patterns.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    pat.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
            }

            PatternKind::Record { path, fields, rest } => {
                path.to_tokens(stream);
                stream.push(Token::new(TokenKind::LBrace, self.span));
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    stream.push(Token::new(
                        TokenKind::Ident(field.name.as_str().to_string().into()),
                        self.span,
                    ));
                    if let Some(pat) = &field.pattern {
                        stream.push(Token::new(TokenKind::Colon, self.span));
                        pat.to_tokens(stream);
                    }
                }
                if *rest {
                    stream.push(Token::new(TokenKind::Comma, self.span));
                    stream.push(Token::new(TokenKind::DotDot, self.span));
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            PatternKind::Rest => {
                stream.push(Token::new(TokenKind::DotDot, self.span));
            }

            PatternKind::Array(patterns) => {
                stream.push(Token::new(TokenKind::LBracket, self.span));
                for (i, pat) in patterns.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    pat.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RBracket, self.span));
            }

            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                stream.push(Token::new(TokenKind::LBracket, self.span));
                for (i, pat) in before.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    pat.to_tokens(stream);
                }
                if let Some(rest_pat) = rest {
                    if !before.is_empty() {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    rest_pat.to_tokens(stream);
                    stream.push(Token::new(TokenKind::DotDot, self.span));
                }
                for pat in after.iter() {
                    stream.push(Token::new(TokenKind::Comma, self.span));
                    pat.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RBracket, self.span));
            }

            PatternKind::Variant { path, data } => {
                path.to_tokens(stream);
                if let Some(variant_data) = data {
                    match variant_data {
                        verum_ast::pattern::VariantPatternData::Tuple(patterns) => {
                            stream.push(Token::new(TokenKind::LParen, self.span));
                            for (i, pat) in patterns.iter().enumerate() {
                                if i > 0 {
                                    stream.push(Token::new(TokenKind::Comma, self.span));
                                }
                                pat.to_tokens(stream);
                            }
                            stream.push(Token::new(TokenKind::RParen, self.span));
                        }
                        verum_ast::pattern::VariantPatternData::Record { fields, rest } => {
                            stream.push(Token::new(TokenKind::LBrace, self.span));
                            for (i, field) in fields.iter().enumerate() {
                                if i > 0 {
                                    stream.push(Token::new(TokenKind::Comma, self.span));
                                }
                                stream.push(Token::new(
                                    TokenKind::Ident(field.name.as_str().to_string().into()),
                                    self.span,
                                ));
                                if let Some(pat) = &field.pattern {
                                    stream.push(Token::new(TokenKind::Colon, self.span));
                                    pat.to_tokens(stream);
                                }
                            }
                            if *rest {
                                if !fields.is_empty() {
                                    stream.push(Token::new(TokenKind::Comma, self.span));
                                }
                                stream.push(Token::new(TokenKind::DotDot, self.span));
                            }
                            stream.push(Token::new(TokenKind::RBrace, self.span));
                        }
                    }
                }
            }

            PatternKind::Or(patterns) => {
                for (i, pat) in patterns.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Pipe, self.span));
                    }
                    pat.to_tokens(stream);
                }
            }

            PatternKind::Reference { mutable, inner } => {
                stream.push(Token::new(TokenKind::Ampersand, self.span));
                if *mutable {
                    stream.push(Token::new(TokenKind::Ident("mut".into()), self.span));
                }
                inner.to_tokens(stream);
            }

            PatternKind::Range {
                start,
                end,
                inclusive,
            } => {
                if let Some(s) = start {
                    match &s.kind {
                        verum_ast::LiteralKind::Int(i) => {
                            stream.push(Token::new(
                                TokenKind::Integer(IntegerLiteral {
                                    raw_value: i.value.to_string().into(),
                                base: 10,
                                    suffix: None,
                                }),
                                self.span,
                            ));
                        }
                        verum_ast::LiteralKind::Char(c) => {
                            stream.push(Token::new(TokenKind::Char(*c), self.span));
                        }
                        _ => {}
                    }
                }
                if *inclusive {
                    stream.push(Token::new(TokenKind::DotDotEq, self.span));
                } else {
                    stream.push(Token::new(TokenKind::DotDot, self.span));
                }
                if let Some(e) = end {
                    match &e.kind {
                        verum_ast::LiteralKind::Int(i) => {
                            stream.push(Token::new(
                                TokenKind::Integer(IntegerLiteral {
                                    raw_value: i.value.to_string().into(),
                                base: 10,
                                    suffix: None,
                                }),
                                self.span,
                            ));
                        }
                        verum_ast::LiteralKind::Char(c) => {
                            stream.push(Token::new(TokenKind::Char(*c), self.span));
                        }
                        _ => {}
                    }
                }
            }

            PatternKind::Paren(inner) => {
                stream.push(Token::new(TokenKind::LParen, self.span));
                inner.to_tokens(stream);
                stream.push(Token::new(TokenKind::RParen, self.span));
            }            PatternKind::View {
                view_function,
                pattern,
            } => {
                view_function.to_tokens(stream);
                stream.push(Token::new(TokenKind::RArrow, self.span));
                pattern.to_tokens(stream);
            }

            PatternKind::Active { name, params, bindings } => {
                // Active pattern support in quote
                // Format: name(params)(bindings) or name() for total patterns
                stream.push(Token::new(
                    TokenKind::Ident(name.as_str().to_string().into()),
                    self.span,
                ));
                // Emit params if present
                if !params.is_empty() {
                    stream.push(Token::new(TokenKind::LParen, self.span));
                    for (i, arg) in params.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Comma, self.span));
                        }
                        arg.to_tokens(stream);
                    }
                    stream.push(Token::new(TokenKind::RParen, self.span));
                }
                // Emit bindings (or empty parens for total patterns)
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, binding) in bindings.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    binding.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
            }

            PatternKind::And(patterns) => {
                // And pattern: pat1 & pat2 & ...
                for (i, pat) in patterns.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Ampersand, self.span));
                    }
                    pat.to_tokens(stream);
                }
            }

            PatternKind::TypeTest { binding, test_type } => {
                // Format: binding is Type
                stream.push(Token::new(
                    TokenKind::Ident(binding.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Is, self.span));
                test_type.to_tokens(stream);
            }

            PatternKind::Stream { head_patterns, rest } => {
                // Format: stream[pat1, pat2, ...rest]
                stream.push(Token::new(TokenKind::Stream, self.span));
                stream.push(Token::new(TokenKind::LBracket, self.span));

                for (i, pat) in head_patterns.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    pat.to_tokens(stream);
                }

                if let verum_common::Maybe::Some(rest_ident) = rest {
                    if !head_patterns.is_empty() {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    stream.push(Token::new(TokenKind::DotDotDot, self.span));
                    stream.push(Token::new(
                        TokenKind::Ident(rest_ident.as_str().to_string().into()),
                        self.span,
                    ));
                }

                stream.push(Token::new(TokenKind::RBracket, self.span));
            }

            PatternKind::Guard { pattern, guard } => {
                // Guard pattern: (pattern if guard)
                // Spec: Rust RFC 3637 - Guard Patterns
                stream.push(Token::new(TokenKind::LParen, self.span));
                pattern.to_tokens(stream);
                stream.push(Token::new(TokenKind::If, self.span));
                guard.to_tokens(stream);
                stream.push(Token::new(TokenKind::RParen, self.span));
            }

            PatternKind::Cons { head, tail } => {
                // Cons pattern: head :: tail
                head.to_tokens(stream);
                stream.push(Token::new(TokenKind::ColonColon, self.span));
                tail.to_tokens(stream);
            }
        }
    }
}

impl ToTokens for Stmt {
    fn to_tokens(&self, stream: &mut TokenStream) {
        use verum_ast::stmt::StmtKind;

        match &self.kind {
            StmtKind::Let { pattern, ty, value } => {
                stream.push(Token::new(TokenKind::Let, self.span));
                pattern.to_tokens(stream);
                if let Some(type_annotation) = ty {
                    stream.push(Token::new(TokenKind::Colon, self.span));
                    type_annotation.to_tokens(stream);
                }
                if let Some(init) = value {
                    stream.push(Token::new(TokenKind::Eq, self.span));
                    init.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::Semicolon, self.span));
            }

            StmtKind::Expr { expr, has_semi } => {
                expr.to_tokens(stream);
                if *has_semi {
                    stream.push(Token::new(TokenKind::Semicolon, self.span));
                }
            }

            StmtKind::Empty => {
                stream.push(Token::new(TokenKind::Semicolon, self.span));
            }

            StmtKind::LetElse {
                pattern,
                ty,
                value,
                else_block,
            } => {
                stream.push(Token::new(TokenKind::Let, self.span));
                pattern.to_tokens(stream);
                if let Some(type_annotation) = ty {
                    stream.push(Token::new(TokenKind::Colon, self.span));
                    type_annotation.to_tokens(stream);
                }
                stream.push(Token::new(TokenKind::Eq, self.span));
                value.to_tokens(stream);
                stream.push(Token::new(TokenKind::Ident("else".into()), self.span));
                else_block.to_tokens(stream);
            }

            StmtKind::Item(item) => {
                item.to_tokens(stream);
            }

            StmtKind::Defer(expr) => {
                stream.push(Token::new(TokenKind::Ident("defer".into()), self.span));
                expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::Semicolon, self.span));
            }

            StmtKind::Errdefer(expr) => {
                stream.push(Token::new(TokenKind::Ident("errdefer".into()), self.span));
                expr.to_tokens(stream);
                stream.push(Token::new(TokenKind::Semicolon, self.span));
            }

            StmtKind::Provide { context, alias, value } => {
                stream.push(Token::new(
                    TokenKind::Ident("provide".into()),
                    self.span,
                ));
                stream.push(Token::new(
                    TokenKind::Ident(context.as_str().to_string().into()),
                    self.span,
                ));
                // Aliased context: `using [Database as source]` enables multiple instances
                // of the same context type with different alias names for disambiguation
                if let verum_common::Maybe::Some(a) = alias {
                    stream.push(Token::new(
                        TokenKind::Ident("as".into()),
                        self.span,
                    ));
                    stream.push(Token::new(
                        TokenKind::Ident(a.as_str().to_string().into()),
                        self.span,
                    ));
                }
                stream.push(Token::new(TokenKind::Eq, self.span));
                value.to_tokens(stream);
                stream.push(Token::new(TokenKind::Semicolon, self.span));
            }

            StmtKind::ProvideScope {
                context,
                alias,
                value,
                block,
            } => {
                stream.push(Token::new(
                    TokenKind::Ident("provide".into()),
                    self.span,
                ));
                stream.push(Token::new(
                    TokenKind::Ident(context.as_str().to_string().into()),
                    self.span,
                ));
                // Aliased context: `using [Database as source]` enables multiple instances
                // of the same context type with different alias names for disambiguation
                if let verum_common::Maybe::Some(a) = alias {
                    stream.push(Token::new(
                        TokenKind::Ident("as".into()),
                        self.span,
                    ));
                    stream.push(Token::new(
                        TokenKind::Ident(a.as_str().to_string().into()),
                        self.span,
                    ));
                }
                stream.push(Token::new(TokenKind::Eq, self.span));
                value.to_tokens(stream);
                stream.push(Token::new(TokenKind::Ident("in".into()), self.span));
                block.to_tokens(stream);
            }
        }
    }
}

impl ToTokens for verum_ast::Item {
    fn to_tokens(&self, stream: &mut TokenStream) {
        use verum_ast::decl::{FunctionParamKind, ItemKind};
        use verum_ast::ty::GenericParamKind;

        // Output attributes first
        for attr in &self.attributes {
            stream.push(Token::new(TokenKind::At, self.span));
            stream.push(Token::new(
                TokenKind::Ident(attr.name.as_str().to_string().into()),
                self.span,
            ));
        }

        match &self.kind {
            ItemKind::Function(func) => {
                // pub async fn name<T>(params) -> RetType { body }
                match func.visibility {
                    verum_ast::decl::Visibility::Public => {
                        stream.push(Token::new(TokenKind::Pub, self.span));
                    }
                    _ => {}
                }
                if func.is_async {
                    stream.push(Token::new(TokenKind::Async, self.span));
                }
                stream.push(Token::new(TokenKind::Fn, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(func.name.as_str().to_string().into()),
                    self.span,
                ));
                // Generics
                if !func.generics.is_empty() {
                    stream.push(Token::new(TokenKind::Lt, self.span));
                    for (i, param) in func.generics.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Comma, self.span));
                        }
                        // Extract name from GenericParamKind
                        let name = match &param.kind {
                            GenericParamKind::Type { name, .. } => name.as_str(),
                            GenericParamKind::HigherKinded { name, .. } => name.as_str(),
                            GenericParamKind::Const { name, .. } => name.as_str(),
                            GenericParamKind::Meta { name, .. } => name.as_str(),
                            GenericParamKind::Lifetime { name } => name.name.as_str(),
                            GenericParamKind::Context { name } => name.name.as_str(),
                            GenericParamKind::Level { name, .. } => name.name.as_str(),
                            GenericParamKind::KindAnnotated { name, .. } => name.as_str(),
                        };
                        stream.push(Token::new(TokenKind::Ident(name.to_string().into()), param.span));
                    }
                    stream.push(Token::new(TokenKind::Gt, self.span));
                }
                // Params
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, param) in func.params.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    match &param.kind {
                        FunctionParamKind::Regular { pattern, ty, .. } => {
                            pattern.to_tokens(stream);
                            stream.push(Token::new(TokenKind::Colon, param.span));
                            ty.to_tokens(stream);
                        }
                        FunctionParamKind::SelfValue => {
                            stream
                                .push(Token::new(TokenKind::Ident("self".into()), param.span));
                        }
                        FunctionParamKind::SelfValueMut => {
                            stream.push(Token::new(TokenKind::Mut, param.span));
                            stream
                                .push(Token::new(TokenKind::Ident("self".into()), param.span));
                        }
                        FunctionParamKind::SelfRef => {
                            stream.push(Token::new(TokenKind::Ampersand, param.span));
                            stream
                                .push(Token::new(TokenKind::Ident("self".into()), param.span));
                        }
                        FunctionParamKind::SelfRefMut => {
                            stream.push(Token::new(TokenKind::Ampersand, param.span));
                            stream.push(Token::new(TokenKind::Mut, param.span));
                            stream
                                .push(Token::new(TokenKind::Ident("self".into()), param.span));
                        }
                        FunctionParamKind::SelfOwn => {
                            // Ownership reference self: %self
                            stream.push(Token::new(TokenKind::Percent, param.span));
                            stream
                                .push(Token::new(TokenKind::Ident("self".into()), param.span));
                        }
                        FunctionParamKind::SelfOwnMut => {
                            // Ownership mutable reference self: %mut self
                            stream.push(Token::new(TokenKind::Percent, param.span));
                            stream.push(Token::new(TokenKind::Mut, param.span));
                            stream
                                .push(Token::new(TokenKind::Ident("self".into()), param.span));
                        }
                        FunctionParamKind::SelfRefChecked => {
                            // CBGR checked self: &checked self
                            stream.push(Token::new(TokenKind::Ampersand, param.span));
                            stream.push(Token::new(TokenKind::Checked, param.span));
                            stream
                                .push(Token::new(TokenKind::Ident("self".into()), param.span));
                        }
                        FunctionParamKind::SelfRefCheckedMut => {
                            // CBGR checked mutable self: &checked mut self
                            stream.push(Token::new(TokenKind::Ampersand, param.span));
                            stream.push(Token::new(TokenKind::Checked, param.span));
                            stream.push(Token::new(TokenKind::Mut, param.span));
                            stream
                                .push(Token::new(TokenKind::Ident("self".into()), param.span));
                        }
                        FunctionParamKind::SelfRefUnsafe => {
                            // CBGR unsafe self: &unsafe self
                            stream.push(Token::new(TokenKind::Ampersand, param.span));
                            stream.push(Token::new(TokenKind::Unsafe, param.span));
                            stream
                                .push(Token::new(TokenKind::Ident("self".into()), param.span));
                        }
                        FunctionParamKind::SelfRefUnsafeMut => {
                            // CBGR unsafe mutable self: &unsafe mut self
                            stream.push(Token::new(TokenKind::Ampersand, param.span));
                            stream.push(Token::new(TokenKind::Unsafe, param.span));
                            stream.push(Token::new(TokenKind::Mut, param.span));
                            stream
                                .push(Token::new(TokenKind::Ident("self".into()), param.span));
                        }
                    }
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
                // Return type
                if let Some(ret_ty) = &func.return_type {
                    stream.push(Token::new(TokenKind::RArrow, self.span));
                    ret_ty.to_tokens(stream);
                }
                // Body - handle FunctionBody
                if let Some(body) = &func.body {
                    match body {
                        verum_ast::FunctionBody::Block(block) => {
                            block.to_tokens(stream);
                        }
                        verum_ast::FunctionBody::Expr(expr) => {
                            stream.push(Token::new(TokenKind::Eq, self.span));
                            expr.to_tokens(stream);
                            stream.push(Token::new(TokenKind::Semicolon, self.span));
                        }
                    }
                }
            }
            ItemKind::Type(type_decl) => {
                stream.push(Token::new(TokenKind::Type, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(type_decl.name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Is, self.span));
                // TypeDecl has body: TypeDeclBody
                stream.push(Token::new(
                    TokenKind::Ident("/* type body */".into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Semicolon, self.span));
            }
            ItemKind::Const(const_decl) => {
                stream.push(Token::new(TokenKind::Const, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(const_decl.name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Colon, self.span));
                const_decl.ty.to_tokens(stream);
                stream.push(Token::new(TokenKind::Eq, self.span));
                const_decl.value.to_tokens(stream);
                stream.push(Token::new(TokenKind::Semicolon, self.span));
            }
            ItemKind::Mount(import) => {
                stream.push(Token::new(TokenKind::Mount, self.span));
                // Emit the import tree
                emit_mount_tree(&import.tree, stream);
                stream.push(Token::new(TokenKind::Semicolon, self.span));
            }
            ItemKind::Module(module) => {
                stream.push(Token::new(TokenKind::Module, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(module.name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::LBrace, self.span));
                // items: Maybe<List<Item>>
                if let Some(items) = &module.items {
                    for item in items.iter() {
                        item.to_tokens(stream);
                    }
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }
            ItemKind::Protocol(proto) => {
                stream.push(Token::new(TokenKind::Type, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(proto.name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Is, self.span));
                stream.push(Token::new(TokenKind::Protocol, self.span));
                stream.push(Token::new(TokenKind::LBrace, self.span));
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }
            ItemKind::Impl(impl_decl) => {
                use verum_ast::decl::ImplKind;
                stream.push(Token::new(TokenKind::Implement, self.span));
                match &impl_decl.kind {
                    ImplKind::Inherent(ty) => {
                        ty.to_tokens(stream);
                    }
                    ImplKind::Protocol {
                        protocol, for_type, ..
                    } => {
                        protocol.to_tokens(stream);
                        stream.push(Token::new(TokenKind::For, self.span));
                        for_type.to_tokens(stream);
                    }
                }
                stream.push(Token::new(TokenKind::LBrace, self.span));
                for item in &impl_decl.items {
                    // Emit visibility if public
                    if matches!(item.visibility, verum_ast::decl::Visibility::Public) {
                        stream.push(Token::new(TokenKind::Pub, item.span));
                    }
                    // Emit impl item based on kind
                    emit_impl_item(&item.kind, item.span, stream);
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }
            ItemKind::Static(static_decl) => {
                stream.push(Token::new(TokenKind::Static, self.span));
                stream.push(Token::new(
                    TokenKind::Ident(static_decl.name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Colon, self.span));
                static_decl.ty.to_tokens(stream);
                stream.push(Token::new(TokenKind::Eq, self.span));
                static_decl.value.to_tokens(stream);
                stream.push(Token::new(TokenKind::Semicolon, self.span));
            }
            ItemKind::Meta(meta_decl) => {
                // meta name!(pattern) { expansion }
                stream.push(Token::new(TokenKind::Ident("meta".into()), self.span));
                stream.push(Token::new(
                    TokenKind::Ident(meta_decl.name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, param) in meta_decl.params.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    stream.push(Token::new(
                        TokenKind::Ident(param.name.as_str().to_string().into()),
                        param.span,
                    ));
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
                stream.push(Token::new(TokenKind::LBrace, self.span));
                for rule in meta_decl.rules.iter() {
                    rule.pattern.to_tokens(stream);
                    stream.push(Token::new(TokenKind::FatArrow, rule.span));
                    rule.expansion.to_tokens(stream);
                    stream.push(Token::new(TokenKind::Semicolon, rule.span));
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }
            ItemKind::Predicate(pred) => {
                // predicate Name(params) -> RetType { body }
                stream.push(Token::new(
                    TokenKind::Ident("predicate".into()),
                    self.span,
                ));
                stream.push(Token::new(
                    TokenKind::Ident(pred.name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, param) in pred.params.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    use verum_ast::decl::FunctionParamKind;
                    if let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                        pattern.to_tokens(stream);
                        stream.push(Token::new(TokenKind::Colon, param.span));
                        ty.to_tokens(stream);
                    }
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
                stream.push(Token::new(TokenKind::RArrow, self.span));
                pred.return_type.to_tokens(stream);
                stream.push(Token::new(TokenKind::LBrace, self.span));
                pred.body.to_tokens(stream);
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }
            ItemKind::Context(ctx_decl) => {
                // context Name { methods }
                if ctx_decl.is_async {
                    stream.push(Token::new(TokenKind::Async, self.span));
                }
                stream.push(Token::new(
                    TokenKind::Ident("context".into()),
                    self.span,
                ));
                stream.push(Token::new(
                    TokenKind::Ident(ctx_decl.name.as_str().to_string().into()),
                    self.span,
                ));
                // Generics
                if !ctx_decl.generics.is_empty() {
                    stream.push(Token::new(TokenKind::Lt, self.span));
                    for (i, param) in ctx_decl.generics.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Comma, self.span));
                        }
                        use verum_ast::ty::GenericParamKind;
                        let name = match &param.kind {
                            GenericParamKind::Type { name, .. } => name.as_str(),
                            GenericParamKind::HigherKinded { name, .. } => name.as_str(),
                            GenericParamKind::Const { name, .. } => name.as_str(),
                            GenericParamKind::Meta { name, .. } => name.as_str(),
                            GenericParamKind::Lifetime { name } => name.name.as_str(),
                            GenericParamKind::Context { name } => name.name.as_str(),
                            GenericParamKind::Level { name, .. } => name.name.as_str(),
                            GenericParamKind::KindAnnotated { name, .. } => name.as_str(),
                        };
                        stream.push(Token::new(TokenKind::Ident(name.to_string().into()), param.span));
                    }
                    stream.push(Token::new(TokenKind::Gt, self.span));
                }
                stream.push(Token::new(TokenKind::LBrace, self.span));
                // Emit method declarations
                for method in ctx_decl.methods.iter() {
                    stream.push(Token::new(TokenKind::Fn, method.span));
                    stream.push(Token::new(
                        TokenKind::Ident(method.name.as_str().to_string().into()),
                        method.span,
                    ));
                    stream.push(Token::new(TokenKind::Semicolon, method.span));
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }
            ItemKind::ContextGroup(group_decl) => {
                // context group Name { Context1, Context2 }
                stream.push(Token::new(
                    TokenKind::Ident("context".into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Ident("group".into()), self.span));
                stream.push(Token::new(
                    TokenKind::Ident(group_decl.name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::Eq, self.span));
                stream.push(Token::new(TokenKind::LBracket, self.span));
                for (i, ctx) in group_decl.contexts.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    // Emit negation if present
                    if ctx.is_negative {
                        stream.push(Token::new(TokenKind::Bang, self.span));
                    }
                    // Emit the context path
                    use verum_ast::ty::PathSegment;
                    for (j, seg) in ctx.path.segments.iter().enumerate() {
                        if j > 0 {
                            stream.push(Token::new(TokenKind::Dot, self.span));
                        }
                        let name = match seg {
                            PathSegment::Name(ident) => ident.name.to_string(),
                            PathSegment::SelfValue => "self".to_string(),
                            PathSegment::Super => "super".to_string(),
                            PathSegment::Cog => "cog".to_string(),
                            PathSegment::Relative => continue, // Skip relative marker
                        };
                        stream.push(Token::new(TokenKind::Ident(name.into()), self.span));
                    }
                    // Emit type args (e.g., Cache<User>)
                    if !ctx.args.is_empty() {
                        stream.push(Token::new(TokenKind::Lt, self.span));
                        for (k, arg) in ctx.args.iter().enumerate() {
                            if k > 0 {
                                stream.push(Token::new(TokenKind::Comma, self.span));
                            }
                            arg.to_tokens(stream);
                        }
                        stream.push(Token::new(TokenKind::Gt, self.span));
                    }
                    // Emit transforms (e.g., .transactional())
                    for transform in ctx.transforms.iter() {
                        stream.push(Token::new(TokenKind::Dot, self.span));
                        stream.push(Token::new(
                            TokenKind::Ident(transform.name.as_str().to_string().into()),
                            self.span,
                        ));
                        stream.push(Token::new(TokenKind::LParen, self.span));
                        for (k, arg) in transform.args.iter().enumerate() {
                            if k > 0 {
                                stream.push(Token::new(TokenKind::Comma, self.span));
                            }
                            arg.to_tokens(stream);
                        }
                        stream.push(Token::new(TokenKind::RParen, self.span));
                    }
                    // Emit alias (e.g., as db)
                    if let verum_common::Maybe::Some(ref alias) = ctx.alias {
                        stream.push(Token::new(TokenKind::As, self.span));
                        stream.push(Token::new(
                            TokenKind::Ident(alias.as_str().to_string().into()),
                            self.span,
                        ));
                    }
                    // Emit condition (e.g., if cfg.enabled)
                    if let verum_common::Maybe::Some(ref condition) = ctx.condition {
                        stream.push(Token::new(TokenKind::If, self.span));
                        condition.to_tokens(stream);
                    }
                }
                stream.push(Token::new(TokenKind::RBracket, self.span));
                stream.push(Token::new(TokenKind::Semicolon, self.span));
            }
            ItemKind::FFIBoundary(ffi_boundary) => {
                // ffi Name { ... }
                stream.push(Token::new(TokenKind::Ident("ffi".into()), self.span));
                stream.push(Token::new(
                    TokenKind::Ident(ffi_boundary.name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::LBrace, self.span));
                // Emit function declarations
                for func in ffi_boundary.functions.iter() {
                    stream.push(Token::new(TokenKind::Fn, func.span));
                    stream.push(Token::new(
                        TokenKind::Ident(func.name.as_str().to_string().into()),
                        func.span,
                    ));
                    stream.push(Token::new(TokenKind::Semicolon, func.span));
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }
            ItemKind::Theorem(theorem_decl)
            | ItemKind::Lemma(theorem_decl)
            | ItemKind::Corollary(theorem_decl) => {
                // theorem/lemma/corollary Name<T>(params): Proposition { proof }
                let keyword = match &self.kind {
                    ItemKind::Theorem(_) => "theorem",
                    ItemKind::Lemma(_) => "lemma",
                    ItemKind::Corollary(_) => "corollary",
                    _ => unreachable!(),
                };
                stream.push(Token::new(TokenKind::Ident(keyword.to_string().into()), self.span));
                stream.push(Token::new(
                    TokenKind::Ident(theorem_decl.name.as_str().to_string().into()),
                    self.span,
                ));
                // Generics
                if !theorem_decl.generics.is_empty() {
                    stream.push(Token::new(TokenKind::Lt, self.span));
                    for (i, param) in theorem_decl.generics.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Comma, self.span));
                        }
                        use verum_ast::ty::GenericParamKind;
                        let name = match &param.kind {
                            GenericParamKind::Type { name, .. } => name.as_str(),
                            GenericParamKind::HigherKinded { name, .. } => name.as_str(),
                            GenericParamKind::Const { name, .. } => name.as_str(),
                            GenericParamKind::Meta { name, .. } => name.as_str(),
                            GenericParamKind::Lifetime { name } => name.name.as_str(),
                            GenericParamKind::Context { name } => name.name.as_str(),
                            GenericParamKind::Level { name, .. } => name.name.as_str(),
                            GenericParamKind::KindAnnotated { name, .. } => name.as_str(),
                        };
                        stream.push(Token::new(TokenKind::Ident(name.to_string().into()), param.span));
                    }
                    stream.push(Token::new(TokenKind::Gt, self.span));
                }
                // Parameters
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, param) in theorem_decl.params.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    use verum_ast::decl::FunctionParamKind;
                    if let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                        pattern.to_tokens(stream);
                        stream.push(Token::new(TokenKind::Colon, param.span));
                        ty.to_tokens(stream);
                    }
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
                stream.push(Token::new(TokenKind::Colon, self.span));
                theorem_decl.proposition.to_tokens(stream);
                // Proof body (if present)
                if let Some(_proof) = &theorem_decl.proof {
                    stream.push(Token::new(TokenKind::LBrace, self.span));
                    // ProofBody tokens would go here - for now just emit placeholder comment
                    stream.push(Token::new(TokenKind::Ident("proof".into()), self.span));
                    stream.push(Token::new(TokenKind::RBrace, self.span));
                }
            }
            ItemKind::Axiom(axiom_decl) => {
                // axiom Name<T>(params): Proposition
                stream.push(Token::new(TokenKind::Ident("axiom".into()), self.span));
                stream.push(Token::new(
                    TokenKind::Ident(axiom_decl.name.as_str().to_string().into()),
                    self.span,
                ));
                // Generics
                if !axiom_decl.generics.is_empty() {
                    stream.push(Token::new(TokenKind::Lt, self.span));
                    for (i, param) in axiom_decl.generics.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Comma, self.span));
                        }
                        use verum_ast::ty::GenericParamKind;
                        let name = match &param.kind {
                            GenericParamKind::Type { name, .. } => name.as_str(),
                            GenericParamKind::HigherKinded { name, .. } => name.as_str(),
                            GenericParamKind::Const { name, .. } => name.as_str(),
                            GenericParamKind::Meta { name, .. } => name.as_str(),
                            GenericParamKind::Lifetime { name } => name.name.as_str(),
                            GenericParamKind::Context { name } => name.name.as_str(),
                            GenericParamKind::Level { name, .. } => name.name.as_str(),
                            GenericParamKind::KindAnnotated { name, .. } => name.as_str(),
                        };
                        stream.push(Token::new(TokenKind::Ident(name.to_string().into()), param.span));
                    }
                    stream.push(Token::new(TokenKind::Gt, self.span));
                }
                // Parameters
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, param) in axiom_decl.params.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    use verum_ast::decl::FunctionParamKind;
                    if let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                        pattern.to_tokens(stream);
                        stream.push(Token::new(TokenKind::Colon, param.span));
                        ty.to_tokens(stream);
                    }
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
                stream.push(Token::new(TokenKind::Colon, self.span));
                axiom_decl.proposition.to_tokens(stream);
                stream.push(Token::new(TokenKind::Semicolon, self.span));
            }
            ItemKind::Tactic(tactic_decl) => {
                // tactic Name(params) is { body }
                stream.push(Token::new(
                    TokenKind::Ident("tactic".into()),
                    self.span,
                ));
                stream.push(Token::new(
                    TokenKind::Ident(tactic_decl.name.as_str().to_string().into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, param) in tactic_decl.params.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    stream.push(Token::new(
                        TokenKind::Ident(param.name.as_str().to_string().into()),
                        param.span,
                    ));
                    stream.push(Token::new(TokenKind::Colon, param.span));
                    // Emit tactic param kind as identifier
                    let kind_str = match &param.kind {
                        verum_ast::decl::TacticParamKind::Expr => "expr",
                        verum_ast::decl::TacticParamKind::Type => "type",
                        verum_ast::decl::TacticParamKind::Tactic => "tactic",
                        verum_ast::decl::TacticParamKind::Hypothesis => "hypothesis",
                        verum_ast::decl::TacticParamKind::Int => "int",
                        verum_ast::decl::TacticParamKind::Prop => "prop",
                        verum_ast::decl::TacticParamKind::Other => "typed",
                    };
                    stream.push(Token::new(
                        TokenKind::Ident(kind_str.to_string().into()),
                        param.span,
                    ));
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
                stream.push(Token::new(TokenKind::Is, self.span));
                stream.push(Token::new(TokenKind::LBrace, self.span));
                // TacticBody tokens - simplified for now
                stream.push(Token::new(
                    TokenKind::Ident("tactic_body".into()),
                    self.span,
                ));
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }
            ItemKind::View(view_decl) => {
                // view Name<T> : ParamType -> ReturnType { constructors }
                stream.push(Token::new(TokenKind::Ident("view".into()), self.span));
                stream.push(Token::new(
                    TokenKind::Ident(view_decl.name.as_str().to_string().into()),
                    self.span,
                ));
                // Generics
                if !view_decl.generics.is_empty() {
                    stream.push(Token::new(TokenKind::Lt, self.span));
                    for (i, param) in view_decl.generics.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Comma, self.span));
                        }
                        use verum_ast::ty::GenericParamKind;
                        let name = match &param.kind {
                            GenericParamKind::Type { name, .. } => name.as_str(),
                            GenericParamKind::HigherKinded { name, .. } => name.as_str(),
                            GenericParamKind::Const { name, .. } => name.as_str(),
                            GenericParamKind::Meta { name, .. } => name.as_str(),
                            GenericParamKind::Lifetime { name } => name.name.as_str(),
                            GenericParamKind::Context { name } => name.name.as_str(),
                            GenericParamKind::Level { name, .. } => name.name.as_str(),
                            GenericParamKind::KindAnnotated { name, .. } => name.as_str(),
                        };
                        stream.push(Token::new(TokenKind::Ident(name.to_string().into()), param.span));
                    }
                    stream.push(Token::new(TokenKind::Gt, self.span));
                }
                stream.push(Token::new(TokenKind::Colon, self.span));
                view_decl.param_type.to_tokens(stream);
                stream.push(Token::new(TokenKind::RArrow, self.span));
                view_decl.return_type.to_tokens(stream);
                stream.push(Token::new(TokenKind::LBrace, self.span));
                for constructor in view_decl.constructors.iter() {
                    stream.push(Token::new(
                        TokenKind::Ident(constructor.name.as_str().to_string().into()),
                        constructor.span,
                    ));
                    // Constructor parameters (FunctionParam type)
                    if !constructor.params.is_empty() {
                        stream.push(Token::new(TokenKind::LParen, constructor.span));
                        for (i, param) in constructor.params.iter().enumerate() {
                            if i > 0 {
                                stream.push(Token::new(TokenKind::Comma, constructor.span));
                            }
                            use verum_ast::decl::FunctionParamKind;
                            if let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                                pattern.to_tokens(stream);
                                stream.push(Token::new(TokenKind::Colon, param.span));
                                ty.to_tokens(stream);
                            }
                        }
                        stream.push(Token::new(TokenKind::RParen, constructor.span));
                    }
                    // Result type
                    stream.push(Token::new(TokenKind::RArrow, constructor.span));
                    constructor.result_type.to_tokens(stream);
                    stream.push(Token::new(TokenKind::Semicolon, constructor.span));
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            ItemKind::ExternBlock(extern_block) => {
                // extern "ABI" { fn foo(); fn bar(); }
                stream.push(Token::new(TokenKind::Extern, self.span));
                if let verum_common::Maybe::Some(ref abi) = extern_block.abi {
                    stream.push(Token::new(TokenKind::Text(abi.clone()), self.span));
                }
                stream.push(Token::new(TokenKind::LBrace, self.span));
                for func in extern_block.functions.iter() {
                    // Functions inside extern block use FunctionDecl, which has its own ToTokens
                    // but we need to handle them specially (no body, just signature + semicolon)
                    match func.visibility {
                        verum_ast::decl::Visibility::Public => {
                            stream.push(Token::new(TokenKind::Pub, func.span));
                        }
                        _ => {}
                    }
                    stream.push(Token::new(TokenKind::Fn, func.span));
                    stream.push(Token::new(TokenKind::Ident(func.name.as_str().to_string().into()), func.span));
                    stream.push(Token::new(TokenKind::LParen, func.span));
                    for (i, param) in func.params.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Comma, func.span));
                        }
                        use verum_ast::decl::FunctionParamKind;
                        if let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                            pattern.to_tokens(stream);
                            stream.push(Token::new(TokenKind::Colon, param.span));
                            ty.to_tokens(stream);
                        }
                    }
                    stream.push(Token::new(TokenKind::RParen, func.span));
                    if let verum_common::Maybe::Some(ref ret_ty) = func.return_type {
                        stream.push(Token::new(TokenKind::RArrow, func.span));
                        ret_ty.to_tokens(stream);
                    }
                    stream.push(Token::new(TokenKind::Semicolon, func.span));
                }
                stream.push(Token::new(TokenKind::RBrace, self.span));
            }

            ItemKind::Pattern(pattern_decl) => {
                // Active pattern declaration:
                // pattern Name(type_params)(params) -> ReturnType = body;
                stream.push(Token::new(TokenKind::Ident("pattern".into()), self.span));
                stream.push(Token::new(
                    TokenKind::Ident(pattern_decl.name.as_str().to_string().into()),
                    self.span,
                ));
                // Type parameters
                if !pattern_decl.type_params.is_empty() {
                    stream.push(Token::new(TokenKind::LParen, self.span));
                    for (i, param) in pattern_decl.type_params.iter().enumerate() {
                        if i > 0 {
                            stream.push(Token::new(TokenKind::Comma, self.span));
                        }
                        if let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                            pattern.to_tokens(stream);
                            stream.push(Token::new(TokenKind::Colon, param.span));
                            ty.to_tokens(stream);
                        }
                    }
                    stream.push(Token::new(TokenKind::RParen, self.span));
                }
                // Pattern parameters
                stream.push(Token::new(TokenKind::LParen, self.span));
                for (i, param) in pattern_decl.params.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, self.span));
                    }
                    if let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                        pattern.to_tokens(stream);
                        stream.push(Token::new(TokenKind::Colon, param.span));
                        ty.to_tokens(stream);
                    }
                }
                stream.push(Token::new(TokenKind::RParen, self.span));
                // Return type
                stream.push(Token::new(TokenKind::RArrow, self.span));
                pattern_decl.return_type.to_tokens(stream);
                // Body
                stream.push(Token::new(TokenKind::Eq, self.span));
                pattern_decl.body.to_tokens(stream);
                stream.push(Token::new(TokenKind::Semicolon, self.span));
            }
            ItemKind::Layer(_) => { /* no-op */ }
        }
    }
}

impl ToTokens for verum_ast::expr::Block {
    fn to_tokens(&self, stream: &mut TokenStream) {
        stream.push(Token::new(TokenKind::LBrace, self.span));
        for stmt in self.stmts.iter() {
            stmt.to_tokens(stream);
        }
        // Handle optional trailing expression (block result)
        if let Some(expr) = &self.expr {
            expr.to_tokens(stream);
        }
        stream.push(Token::new(TokenKind::RBrace, self.span));
    }
}

impl ToTokens for verum_ast::pattern::MatchArm {
    fn to_tokens(&self, stream: &mut TokenStream) {
        self.pattern.to_tokens(stream);
        if let Some(guard) = &self.guard {
            stream.push(Token::new(TokenKind::If, self.span));
            guard.to_tokens(stream);
        }
        stream.push(Token::new(TokenKind::FatArrow, self.span));
        self.body.to_tokens(stream);
        stream.push(Token::new(TokenKind::Comma, self.span));
    }
}

impl ToTokens for verum_ast::expr::IfCondition {
    fn to_tokens(&self, stream: &mut TokenStream) {
        use verum_ast::expr::ConditionKind;
        for (i, condition) in self.conditions.iter().enumerate() {
            if i > 0 {
                stream.push(Token::new(TokenKind::AmpersandAmpersand, self.span));
            }
            match condition {
                ConditionKind::Expr(expr) => {
                    expr.to_tokens(stream);
                }
                ConditionKind::Let { pattern, value } => {
                    stream.push(Token::new(TokenKind::Let, self.span));
                    pattern.to_tokens(stream);
                    stream.push(Token::new(TokenKind::Eq, self.span));
                    value.to_tokens(stream);
                }
            }
        }
    }
}

impl ToTokens for verum_ast::ty::Path {
    fn to_tokens(&self, stream: &mut TokenStream) {
        if let Some(ident) = self.as_ident() {
            stream.push(Token::new(
                TokenKind::Ident(ident.as_str().to_string().into()),
                self.span,
            ));
        } else {
            for (i, segment) in self.segments.iter().enumerate() {
                if i > 0 {
                    stream.push(Token::new(TokenKind::Dot, self.span));
                }
                if let verum_ast::ty::PathSegment::Name(ident) = segment {
                    stream.push(Token::new(
                        TokenKind::Ident(ident.as_str().to_string().into()),
                        self.span,
                    ));
                }
            }
        }
    }
}

// ============================================================================
// Helper functions for ToTokens implementations
// ============================================================================

/// Emit a mount tree as tokens.
///
/// Handles all MountTreeKind variants:
/// - Path: simple import like `std.io.File`
/// - Glob: glob import like `std.io.*`
/// - Nested: nested imports like `std.io.{File, Read, Write}`
fn emit_mount_tree(tree: &verum_ast::decl::MountTree, stream: &mut TokenStream) {
    use verum_ast::decl::MountTreeKind;

    match &tree.kind {
        MountTreeKind::Path(path) => {
            path.to_tokens(stream);
        }
        MountTreeKind::Glob(path) => {
            path.to_tokens(stream);
            stream.push(Token::new(TokenKind::Dot, tree.span));
            stream.push(Token::new(TokenKind::Star, tree.span));
        }
        MountTreeKind::Nested { prefix, trees } => {
            prefix.to_tokens(stream);
            stream.push(Token::new(TokenKind::Dot, tree.span));
            stream.push(Token::new(TokenKind::LBrace, tree.span));
            for (i, subtree) in trees.iter().enumerate() {
                if i > 0 {
                    stream.push(Token::new(TokenKind::Comma, tree.span));
                }
                emit_mount_tree(subtree, stream);
            }
            stream.push(Token::new(TokenKind::RBrace, tree.span));
        }
    }
}

/// Emit an impl item (function, type alias, or const) as tokens.
fn emit_impl_item(kind: &verum_ast::decl::ImplItemKind, span: Span, stream: &mut TokenStream) {
    use verum_ast::decl::ImplItemKind;

    match kind {
        ImplItemKind::Function(func) => {
            // Emit the full function declaration
            if func.is_async {
                stream.push(Token::new(TokenKind::Async, span));
            }
            stream.push(Token::new(TokenKind::Fn, span));
            stream.push(Token::new(
                TokenKind::Ident(func.name.as_str().to_string().into()),
                span,
            ));
            // Generics
            if !func.generics.is_empty() {
                stream.push(Token::new(TokenKind::Lt, span));
                for (i, param) in func.generics.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, span));
                    }
                    use verum_ast::ty::GenericParamKind;
                    let name = match &param.kind {
                        GenericParamKind::Type { name, .. } => name.as_str(),
                        GenericParamKind::HigherKinded { name, .. } => name.as_str(),
                        GenericParamKind::Const { name, .. } => name.as_str(),
                        GenericParamKind::Meta { name, .. } => name.as_str(),
                        GenericParamKind::Lifetime { name } => name.name.as_str(),
                        GenericParamKind::Context { name } => name.name.as_str(),
                        GenericParamKind::Level { name } => name.name.as_str(),
                        GenericParamKind::KindAnnotated { name, .. } => name.as_str(),
                    };
                    stream.push(Token::new(TokenKind::Ident(name.to_string().into()), param.span));
                }
                stream.push(Token::new(TokenKind::Gt, span));
            }
            // Params
            stream.push(Token::new(TokenKind::LParen, span));
            for (i, param) in func.params.iter().enumerate() {
                if i > 0 {
                    stream.push(Token::new(TokenKind::Comma, span));
                }
                use verum_ast::decl::FunctionParamKind;
                match &param.kind {
                    FunctionParamKind::Regular { pattern, ty, .. } => {
                        pattern.to_tokens(stream);
                        stream.push(Token::new(TokenKind::Colon, param.span));
                        ty.to_tokens(stream);
                    }
                    FunctionParamKind::SelfValue => {
                        stream.push(Token::new(TokenKind::Ident("self".into()), param.span));
                    }
                    FunctionParamKind::SelfValueMut => {
                        stream.push(Token::new(TokenKind::Mut, param.span));
                        stream.push(Token::new(TokenKind::Ident("self".into()), param.span));
                    }
                    FunctionParamKind::SelfRef => {
                        stream.push(Token::new(TokenKind::Ampersand, param.span));
                        stream.push(Token::new(TokenKind::Ident("self".into()), param.span));
                    }
                    FunctionParamKind::SelfRefMut => {
                        stream.push(Token::new(TokenKind::Ampersand, param.span));
                        stream.push(Token::new(TokenKind::Mut, param.span));
                        stream.push(Token::new(TokenKind::Ident("self".into()), param.span));
                    }
                    FunctionParamKind::SelfOwn => {
                        stream.push(Token::new(TokenKind::Percent, param.span));
                        stream.push(Token::new(TokenKind::Ident("self".into()), param.span));
                    }
                    FunctionParamKind::SelfOwnMut => {
                        stream.push(Token::new(TokenKind::Percent, param.span));
                        stream.push(Token::new(TokenKind::Mut, param.span));
                        stream.push(Token::new(TokenKind::Ident("self".into()), param.span));
                    }
                    FunctionParamKind::SelfRefChecked => {
                        stream.push(Token::new(TokenKind::Ampersand, param.span));
                        stream.push(Token::new(TokenKind::Checked, param.span));
                        stream.push(Token::new(TokenKind::Ident("self".into()), param.span));
                    }
                    FunctionParamKind::SelfRefCheckedMut => {
                        stream.push(Token::new(TokenKind::Ampersand, param.span));
                        stream.push(Token::new(TokenKind::Checked, param.span));
                        stream.push(Token::new(TokenKind::Mut, param.span));
                        stream.push(Token::new(TokenKind::Ident("self".into()), param.span));
                    }
                    FunctionParamKind::SelfRefUnsafe => {
                        stream.push(Token::new(TokenKind::Ampersand, param.span));
                        stream.push(Token::new(TokenKind::Unsafe, param.span));
                        stream.push(Token::new(TokenKind::Ident("self".into()), param.span));
                    }
                    FunctionParamKind::SelfRefUnsafeMut => {
                        stream.push(Token::new(TokenKind::Ampersand, param.span));
                        stream.push(Token::new(TokenKind::Unsafe, param.span));
                        stream.push(Token::new(TokenKind::Mut, param.span));
                        stream.push(Token::new(TokenKind::Ident("self".into()), param.span));
                    }
                }
            }
            stream.push(Token::new(TokenKind::RParen, span));
            // Return type
            if let Some(ret_ty) = &func.return_type {
                stream.push(Token::new(TokenKind::RArrow, span));
                ret_ty.to_tokens(stream);
            }
            // Body
            if let Some(body) = &func.body {
                match body {
                    verum_ast::FunctionBody::Block(block) => {
                        block.to_tokens(stream);
                    }
                    verum_ast::FunctionBody::Expr(expr) => {
                        stream.push(Token::new(TokenKind::Eq, span));
                        expr.to_tokens(stream);
                        stream.push(Token::new(TokenKind::Semicolon, span));
                    }
                }
            }
        }
        ImplItemKind::Type {
            name,
            type_params,
            ty,
        } => {
            // type Name<Params> = Type;
            stream.push(Token::new(TokenKind::Type, span));
            stream.push(Token::new(
                TokenKind::Ident(name.as_str().to_string().into()),
                span,
            ));
            // GAT type params if any
            if !type_params.is_empty() {
                stream.push(Token::new(TokenKind::Lt, span));
                for (i, param) in type_params.iter().enumerate() {
                    if i > 0 {
                        stream.push(Token::new(TokenKind::Comma, span));
                    }
                    use verum_ast::ty::GenericParamKind;
                    let name = match &param.kind {
                        GenericParamKind::Type { name, .. } => name.as_str(),
                        GenericParamKind::HigherKinded { name, .. } => name.as_str(),
                        GenericParamKind::Const { name, .. } => name.as_str(),
                        GenericParamKind::Meta { name, .. } => name.as_str(),
                        GenericParamKind::Lifetime { name } => name.name.as_str(),
                        GenericParamKind::Context { name } => name.name.as_str(),
                        GenericParamKind::Level { name } => name.name.as_str(),
                        GenericParamKind::KindAnnotated { name, .. } => name.as_str(),
                    };
                    stream.push(Token::new(TokenKind::Ident(name.to_string().into()), param.span));
                }
                stream.push(Token::new(TokenKind::Gt, span));
            }
            stream.push(Token::new(TokenKind::Eq, span));
            ty.to_tokens(stream);
            stream.push(Token::new(TokenKind::Semicolon, span));
        }
        ImplItemKind::Const { name, ty, value } => {
            // const NAME: Type = value;
            stream.push(Token::new(TokenKind::Const, span));
            stream.push(Token::new(
                TokenKind::Ident(name.as_str().to_string().into()),
                span,
            ));
            stream.push(Token::new(TokenKind::Colon, span));
            ty.to_tokens(stream);
            stream.push(Token::new(TokenKind::Eq, span));
            value.to_tokens(stream);
            stream.push(Token::new(TokenKind::Semicolon, span));
        }
        ImplItemKind::Proof { axiom_name, tactic: _ } => {
            // `proof axiom_name by /* tactic */ ;` — quote preserves
            // the keyword sequence; full tactic quotation flows through
            // the tactic-expr quote path (not yet expanded here to
            // avoid the recursive import in this file).
            stream.push(Token::new(TokenKind::Ident(Text::from("proof")), span));
            stream.push(Token::new(
                TokenKind::Ident(Text::from(axiom_name.as_str().to_string())),
                span,
            ));
            stream.push(Token::new(TokenKind::Ident(Text::from("by")), span));
            stream.push(Token::new(TokenKind::Ident(Text::from("auto")), span));
            stream.push(Token::new(TokenKind::Semicolon, span));
        }
    }
}

// ============================================================================
// Parse Errors
// ============================================================================

/// Errors that can occur during token stream parsing
#[derive(Debug, Clone)]
pub enum ParseError {
    /// The token stream is empty
    EmptyTokenStream,

    /// Parse error with message
    ParseFailed(Text),

    /// Feature not yet implemented
    NotImplemented(Text),

    /// Tokens remain after parsing completed
    UnconsumedTokens {
        /// Number of unconsumed tokens
        count: usize,
        /// First unconsumed token description
        first_token: Text,
    },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::EmptyTokenStream => write!(f, "Cannot parse empty token stream"),
            ParseError::ParseFailed(msg) => write!(f, "Parse error: {}", msg.as_str()),
            ParseError::NotImplemented(msg) => write!(f, "Not implemented: {}", msg.as_str()),
            ParseError::UnconsumedTokens { count, first_token } => write!(
                f,
                "Unconsumed tokens: {} token(s) remaining, starting with {}",
                count,
                first_token.as_str()
            ),
        }
    }
}

impl std::error::Error for ParseError {}

/// Helper function to create a simple identifier token stream
pub fn ident(name: &str, span: Span) -> TokenStream {
    let mut stream = TokenStream::new();
    stream.push(Token::new(TokenKind::Ident(name.to_string().into()), span));
    stream
}

/// Helper function to create a simple literal token stream
pub fn literal_int(value: i64, span: Span) -> TokenStream {
    let mut stream = TokenStream::new();
    stream.push(Token::new(
        TokenKind::Integer(IntegerLiteral {
            raw_value: value.to_string().into(),
            base: 10,
            suffix: None,
        }),
        span,
    ));
    stream
}

/// Helper function to create a simple string literal token stream
pub fn literal_string(value: &str, span: Span) -> TokenStream {
    let mut stream = TokenStream::new();
    stream.push(Token::new(TokenKind::Text(value.to_string().into()), span));
    stream
}

// ============================================================================
// Quote Builder - Programmatic TokenStream construction
// ============================================================================

/// Builder for constructing token streams programmatically
///
/// This provides a fluent API for building token streams with proper
/// hygiene and interpolation support, similar to quote! in Rust proc macros.
///
/// # Example
///
/// ```ignore
/// let builder = QuoteBuilder::new();
/// let stream = builder
///     .ident("implement")
///     .ident("Debug")
///     .keyword("for")
///     .ident(&type_name)
///     .punct("{")
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct QuoteBuilder {
    /// The token stream being built
    stream: TokenStream,
    /// Source span for all generated tokens
    span: Span,
    /// Hygiene counter for unique identifiers
    hygiene_counter: u64,
}

impl QuoteBuilder {
    /// Create a new quote builder
    pub fn new() -> Self {
        Self {
            stream: TokenStream::new(),
            span: Span::default(),
            hygiene_counter: 0,
        }
    }

    /// Create a quote builder with a specific span
    pub fn with_span(span: Span) -> Self {
        Self {
            stream: TokenStream::new().with_span(span),
            span,
            hygiene_counter: 0,
        }
    }

    /// Add an identifier token
    pub fn ident(mut self, name: &str) -> Self {
        self.stream
            .push(Token::new(TokenKind::Ident(name.to_string().into()), self.span));
        self
    }

    /// Add a hygienic identifier (prevents capture from outer scope)
    ///
    /// Generated identifiers have a unique suffix to prevent variable capture.
    pub fn hygienic_ident(mut self, name: &str) -> Self {
        self.hygiene_counter += 1;
        let hygienic_name = format!("{}__{}", name, self.hygiene_counter);
        self.stream
            .push(Token::new(TokenKind::Ident(hygienic_name.into()), self.span));
        self
    }

    /// Add a keyword token
    pub fn keyword(mut self, kw: &str) -> Self {
        let kind = match kw {
            "fn" => TokenKind::Fn,
            "let" => TokenKind::Let,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "match" => TokenKind::Match,
            "for" => TokenKind::For,
            "while" => TokenKind::While,
            "loop" => TokenKind::Loop,
            "return" => TokenKind::Return,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "type" => TokenKind::Type,
            "implement" => TokenKind::Implement,
            "protocol" => TokenKind::Protocol,
            "where" => TokenKind::Where,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "self" => TokenKind::SelfValue,
            "Self" => TokenKind::SelfType,
            "is" => TokenKind::Is,
            "as" => TokenKind::As,
            "in" => TokenKind::In,
            "using" => TokenKind::Using,
            "async" => TokenKind::Async,
            "await" => TokenKind::Await,
            "meta" => TokenKind::Meta,
            _ => TokenKind::Ident(kw.to_string().into()),
        };
        self.stream.push(Token::new(kind, self.span));
        self
    }

    /// Add punctuation
    pub fn punct(mut self, p: &str) -> Self {
        let kind = match p {
            "(" => TokenKind::LParen,
            ")" => TokenKind::RParen,
            "{" => TokenKind::LBrace,
            "}" => TokenKind::RBrace,
            "[" => TokenKind::LBracket,
            "]" => TokenKind::RBracket,
            "," => TokenKind::Comma,
            ";" => TokenKind::Semicolon,
            ":" => TokenKind::Colon,
            "." => TokenKind::Dot,
            ".." => TokenKind::DotDot,
            "..=" => TokenKind::DotDotEq,
            "->" => TokenKind::RArrow,
            "=>" => TokenKind::FatArrow,
            "=" => TokenKind::Eq,
            "==" => TokenKind::EqEq,
            "!=" => TokenKind::BangEq,
            "<" => TokenKind::Lt,
            ">" => TokenKind::Gt,
            "<=" => TokenKind::LtEq,
            ">=" => TokenKind::GtEq,
            "+" => TokenKind::Plus,
            "-" => TokenKind::Minus,
            "*" => TokenKind::Star,
            "/" => TokenKind::Slash,
            "%" => TokenKind::Percent,
            "&" => TokenKind::Ampersand,
            "&&" => TokenKind::AmpersandAmpersand,
            "|" => TokenKind::Pipe,
            "||" => TokenKind::PipePipe,
            "!" => TokenKind::Bang,
            "?" => TokenKind::Question,
            "@" => TokenKind::At,
            "#" => TokenKind::Hash,
            _ => TokenKind::Ident(p.to_string().into()), // Fallback
        };
        self.stream.push(Token::new(kind, self.span));
        self
    }

    /// Add an integer literal
    pub fn int(mut self, value: i64) -> Self {
        self.stream.push(Token::new(
            TokenKind::Integer(IntegerLiteral {
                raw_value: value.to_string().into(),
                base: 10,
                suffix: None,
            }),
            self.span,
        ));
        self
    }

    /// Add a float literal
    pub fn float(mut self, value: f64) -> Self {
        self.stream.push(Token::new(
            TokenKind::Float(FloatLiteral {
                value,
                suffix: None,
                raw: format!("{}", value).into(),
            }),
            self.span,
        ));
        self
    }

    /// Add a string literal
    pub fn string(mut self, value: &str) -> Self {
        self.stream
            .push(Token::new(TokenKind::Text(value.to_string().into()), self.span));
        self
    }

    /// Add a boolean literal
    pub fn boolean(mut self, value: bool) -> Self {
        let kind = if value {
            TokenKind::True
        } else {
            TokenKind::False
        };
        self.stream.push(Token::new(kind, self.span));
        self
    }

    /// Interpolate another token stream (unquote)
    ///
    /// This is equivalent to #var in quote! syntax.
    pub fn interpolate(mut self, stream: TokenStream) -> Self {
        self.stream.extend(stream);
        self
    }

    /// Interpolate an expression
    pub fn interpolate_expr(mut self, expr: &Expr) -> Self {
        expr.to_tokens(&mut self.stream);
        self
    }

    /// Interpolate a type
    pub fn interpolate_type(mut self, ty: &Type) -> Self {
        ty.to_tokens(&mut self.stream);
        self
    }

    /// Interpolate with repetition (equivalent to #(...)* )
    ///
    /// Generates tokens for each item in the iterator, optionally with a separator.
    pub fn repeat<I, F>(mut self, items: I, separator: Option<&str>, mut generator: F) -> Self
    where
        I: IntoIterator,
        F: FnMut(I::Item) -> TokenStream,
    {
        let items: Vec<_> = items.into_iter().collect();
        for (i, item) in items.into_iter().enumerate() {
            if i > 0 {
                if let Some(sep) = separator {
                    self = self.punct(sep);
                }
            }
            let tokens = generator(item);
            self.stream.extend(tokens);
        }
        self
    }

    /// Conditionally add tokens (equivalent to #(...)?  )
    pub fn optional<F>(mut self, condition: bool, generator: F) -> Self
    where
        F: FnOnce() -> TokenStream,
    {
        if condition {
            self.stream.extend(generator());
        }
        self
    }

    /// Add a group (parenthesized, braced, or bracketed)
    pub fn group(mut self, delimiter: GroupDelimiter, contents: TokenStream) -> Self {
        let (open, close) = match delimiter {
            GroupDelimiter::Parenthesis => (TokenKind::LParen, TokenKind::RParen),
            GroupDelimiter::Brace => (TokenKind::LBrace, TokenKind::RBrace),
            GroupDelimiter::Bracket => (TokenKind::LBracket, TokenKind::RBracket),
        };
        self.stream.push(Token::new(open, self.span));
        self.stream.extend(contents);
        self.stream.push(Token::new(close, self.span));
        self
    }

    /// Build the final token stream
    pub fn build(self) -> TokenStream {
        self.stream
    }
}

impl Default for QuoteBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Delimiter types for groups
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupDelimiter {
    /// ( )
    Parenthesis,
    /// { }
    Brace,
    /// [ ]
    Bracket,
}

// ============================================================================
// Stringify and format helpers
// ============================================================================

/// Create an identifier from a string (stringify equivalent)
pub fn stringify(s: &str, span: Span) -> TokenStream {
    let mut stream = TokenStream::new();
    stream.push(Token::new(TokenKind::Text(s.to_string().into()), span));
    stream
}

/// Concatenate multiple strings at compile-time
pub fn concat(parts: &[&str], span: Span) -> TokenStream {
    let concatenated: String = parts.iter().copied().collect();
    let mut stream = TokenStream::new();
    stream.push(Token::new(TokenKind::Text(concatenated.into()), span));
    stream
}

/// Create an identifier with a formatted name
///
/// Supports both positional and named placeholders:
/// - `{}` - Positional placeholder, replaced with args in order
/// - `{0}`, `{1}`, etc. - Indexed placeholder
/// - `{name}` - Named placeholder (must provide name=value pairs via format_ident_named)
///
/// # Examples
///
/// ```ignore
/// // Positional: "get_{}_{}" with ["user", "id"] -> "get_user_id"
/// // Indexed: "{1}_{0}" with ["suffix", "prefix"] -> "prefix_suffix"
/// ```
pub fn format_ident(format: &str, args: &[&str], span: Span) -> TokenStream {
    let mut result =
        String::with_capacity(format.len() + args.iter().map(|s| s.len()).sum::<usize>());
    let mut chars = format.chars().peekable();
    let mut positional_index = 0;

    while let Some(ch) = chars.next() {
        if ch == '{' {
            // Check for escaped brace
            if chars.peek() == Some(&'{') {
                result.push('{');
                chars.next();
                continue;
            }

            // Parse placeholder content
            let mut placeholder = String::new();
            while let Some(&c) = chars.peek() {
                if c == '}' {
                    chars.next();
                    break;
                }
                placeholder.push(c);
                chars.next();
            }

            if placeholder.is_empty() {
                // Positional placeholder {}
                if positional_index < args.len() {
                    result.push_str(args[positional_index]);
                    positional_index += 1;
                }
            } else if let Ok(index) = placeholder.parse::<usize>() {
                // Indexed placeholder {0}, {1}, etc.
                if index < args.len() {
                    result.push_str(args[index]);
                }
            } else {
                // Named placeholder - in basic format_ident, just keep as-is
                // For named support, use format_ident_named
                result.push('{');
                result.push_str(&placeholder);
                result.push('}');
            }
        } else if ch == '}' {
            // Check for escaped brace
            if chars.peek() == Some(&'}') {
                result.push('}');
                chars.next();
            } else {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }

    ident(&result, span)
}

/// Create an identifier with named placeholders
///
/// Supports named placeholders like `{name}` which are replaced with
/// corresponding values from the args slice of (name, value) pairs.
///
/// # Examples
///
/// ```ignore
/// // "{type}_{field}" with [("type", "User"), ("field", "name")] -> "User_name"
/// ```
pub fn format_ident_named(format: &str, args: &[(&str, &str)], span: Span) -> TokenStream {
    let mut result =
        String::with_capacity(format.len() + args.iter().map(|(_, v)| v.len()).sum::<usize>());
    let mut chars = format.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' {
            // Check for escaped brace
            if chars.peek() == Some(&'{') {
                result.push('{');
                chars.next();
                continue;
            }

            // Parse placeholder name
            let mut name = String::new();
            while let Some(&c) = chars.peek() {
                if c == '}' {
                    chars.next();
                    break;
                }
                name.push(c);
                chars.next();
            }

            // Look up named argument
            let found = args.iter().find(|(n, _)| *n == name).map(|(_, v)| *v);
            if let Some(value) = found {
                result.push_str(value);
            } else {
                // Keep placeholder if not found
                result.push('{');
                result.push_str(&name);
                result.push('}');
            }
        } else if ch == '}' {
            // Check for escaped brace
            if chars.peek() == Some(&'}') {
                result.push('}');
                chars.next();
            } else {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }

    ident(&result, span)
}

// ============================================================================
// Additional ToTokens implementations
// ============================================================================

impl ToTokens for Text {
    fn to_tokens(&self, stream: &mut TokenStream) {
        stream.push(Token::new(
            TokenKind::Text(self.as_str().to_string().into()),
            Span::default(),
        ));
    }
}

impl ToTokens for i64 {
    fn to_tokens(&self, stream: &mut TokenStream) {
        stream.push(Token::new(
            TokenKind::Integer(IntegerLiteral {
                raw_value: self.to_string().into(),
                base: 10,
                suffix: None,
            }),
            Span::default(),
        ));
    }
}

impl ToTokens for f64 {
    fn to_tokens(&self, stream: &mut TokenStream) {
        stream.push(Token::new(
            TokenKind::Float(FloatLiteral {
                value: *self,
                suffix: None,
                raw: format!("{}", self).into(),
            }),
            Span::default(),
        ));
    }
}

impl ToTokens for bool {
    fn to_tokens(&self, stream: &mut TokenStream) {
        let kind = if *self {
            TokenKind::True
        } else {
            TokenKind::False
        };
        stream.push(Token::new(kind, Span::default()));
    }
}

impl ToTokens for TokenStream {
    fn to_tokens(&self, stream: &mut TokenStream) {
        stream.extend(self.clone());
    }
}

impl<T: ToTokens> ToTokens for List<T> {
    fn to_tokens(&self, stream: &mut TokenStream) {
        for item in self.iter() {
            item.to_tokens(stream);
        }
    }
}

impl<T: ToTokens> ToTokens for Maybe<T> {
    fn to_tokens(&self, stream: &mut TokenStream) {
        if let Maybe::Some(inner) = self {
            inner.to_tokens(stream);
        }
    }
}

// ============================================================================
// Quote! Macro Implementation
// ============================================================================

/// A parsed quote! invocation with interpolation support
///
/// This struct represents a quasi-quotation that can contain interpolated
/// variables and repetition patterns. It parses the quote! syntax and can
/// expand it with provided context.
///
/// # Syntax
///
/// - `#name` - Single interpolation (substitutes a variable)
/// - `#(#name),*` - Repetition with comma separator
/// - `#(#name)*` - Repetition without separator
/// - `#(#name);+` - Repetition with semicolon separator (at least one)
#[derive(Debug, Clone)]
pub struct Quote {
    /// The parsed tokens with interpolation markers
    tokens: List<QuoteToken>,
}

/// A token in a quote! invocation
#[derive(Debug, Clone)]
enum QuoteToken {
    /// Regular token (not interpolated)
    Token(Token),
    /// Single interpolation: #name
    Interpolation(Text),
    /// Repetition: #(#name),*
    Repetition {
        /// The pattern tokens (may contain interpolations)
        pattern: List<QuoteToken>,
        /// Separator between repetitions (e.g., comma, semicolon)
        separator: Maybe<TokenKind>,
        /// At least one repetition required (+ vs *)
        at_least_one: bool,
    },
}

/// Kind of interpolation in a quote
#[derive(Debug, Clone, PartialEq)]
pub enum InterpolationKind {
    /// Single variable interpolation: #name
    Single(Text),
    /// Repeated interpolation: #(#name),*
    Repeat {
        /// The variable name to repeat
        var: Text,
        /// Optional separator token
        separator: Maybe<TokenKind>,
    },
}

/// Context for expanding quote! macros
///
/// This holds the values to be interpolated into the quasi-quotation.
#[derive(Debug, Clone, Default)]
pub struct MetaContext {
    /// Single value bindings
    singles: Map<Text, TokenStream>,
    /// Repeated value bindings (for #(...)* patterns)
    repeats: Map<Text, List<TokenStream>>,
}

impl MetaContext {
    /// Create a new empty context
    pub fn new() -> Self {
        Self {
            singles: Map::new(),
            repeats: Map::new(),
        }
    }

    /// Bind a single value for interpolation
    pub fn bind_single(&mut self, name: Text, value: TokenStream) {
        self.singles.insert(name, value);
    }

    /// Bind a repeated value for interpolation
    pub fn bind_repeat(&mut self, name: Text, values: List<TokenStream>) {
        self.repeats.insert(name, values);
    }

    /// Get a single binding
    pub fn get_single(&self, name: &str) -> Maybe<&TokenStream> {
        match self.singles.get(&Text::from(name)) {
            Some(v) => Maybe::Some(v),
            None => Maybe::None,
        }
    }

    /// Get a repeat binding
    pub fn get_repeat(&self, name: &str) -> Maybe<&List<TokenStream>> {
        match self.repeats.get(&Text::from(name)) {
            Some(v) => Maybe::Some(v),
            None => Maybe::None,
        }
    }
}

/// Errors that can occur during quote! parsing or expansion
#[derive(Debug, Clone)]
pub enum QuoteError {
    /// Failed to parse the quote syntax
    ParseError(Text),
    /// Interpolation variable not found in context
    UnboundVariable(Text),
    /// Invalid interpolation syntax
    InvalidInterpolation(Text),
    /// Repetition pattern mismatch
    RepetitionMismatch(Text),
}

impl std::fmt::Display for QuoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QuoteError::ParseError(msg) => write!(f, "Parse error: {}", msg.as_str()),
            QuoteError::UnboundVariable(var) => {
                write!(f, "Unbound interpolation variable: {}", var.as_str())
            }
            QuoteError::InvalidInterpolation(msg) => {
                write!(f, "Invalid interpolation: {}", msg.as_str())
            }
            QuoteError::RepetitionMismatch(msg) => {
                write!(f, "Repetition mismatch: {}", msg.as_str())
            }
        }
    }
}

impl std::error::Error for QuoteError {}

impl Quote {
    /// Parse a quote! invocation from a string
    ///
    /// This parses the quasi-quotation syntax, identifying interpolation
    /// points and repetition patterns.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let quote = Quote::parse("let #name = #value;")?;
    /// ```
    pub fn parse(input: &str) -> Result<Self, QuoteError> {
        let file_id = verum_ast::FileId::new(0);
        let tokens = Self::lex_with_interpolations(input, file_id)?;
        Ok(Self { tokens })
    }

    /// Lex the input and identify interpolation patterns
    fn lex_with_interpolations(
        input: &str,
        file_id: verum_ast::FileId,
    ) -> Result<List<QuoteToken>, QuoteError> {
        use verum_lexer::Lexer;

        let lexer = Lexer::new(input, file_id);
        let mut result = List::new();
        let tokens: std::vec::Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut i = 0;

        while i < tokens.len() {
            // Check for interpolation (#)
            if i + 1 < tokens.len() && matches!(tokens[i].kind, TokenKind::Hash) {
                i += 1;

                // Check for repetition pattern: #(...)
                if matches!(tokens[i].kind, TokenKind::LParen) {
                    let rep = Self::parse_repetition(&tokens, &mut i)?;
                    result.push(QuoteToken::Repetition {
                        pattern: rep.pattern,
                        separator: rep.separator,
                        at_least_one: rep.at_least_one,
                    });
                } else if let TokenKind::Ident(ref name) = tokens[i].kind {
                    // Single interpolation: #name
                    result.push(QuoteToken::Interpolation(Text::from(name.as_str())));
                    i += 1;
                } else {
                    return Err(QuoteError::InvalidInterpolation(Text::from(
                        "Expected identifier or '(' after '#'",
                    )));
                }
            } else {
                result.push(QuoteToken::Token(tokens[i].clone()));
                i += 1;
            }
        }

        Ok(result)
    }

    /// Parse a repetition pattern: #(...),* or #(...)+
    fn parse_repetition(tokens: &[Token], i: &mut usize) -> Result<RepetitionPattern, QuoteError> {
        // Skip the opening paren
        *i += 1;
        let _start = *i;

        // Find matching closing paren
        let mut depth = 1;
        let mut pattern_tokens = Vec::new();

        while *i < tokens.len() && depth > 0 {
            match tokens[*i].kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            pattern_tokens.push(tokens[*i].clone());
            *i += 1;
        }

        if depth != 0 {
            return Err(QuoteError::ParseError(Text::from(
                "Unmatched parentheses in repetition pattern",
            )));
        }

        // Skip closing paren
        *i += 1;

        // Parse the pattern content recursively
        let pattern = Self::lex_pattern_tokens(&pattern_tokens)?;

        // Check for separator and repetition marker (,* or ;+ etc.)
        let (separator, at_least_one) = if *i < tokens.len() {
            let sep = match &tokens[*i].kind {
                TokenKind::Comma => {
                    let sep = Some(TokenKind::Comma);
                    *i += 1;
                    sep
                }
                TokenKind::Semicolon => {
                    let sep = Some(TokenKind::Semicolon);
                    *i += 1;
                    sep
                }
                _ => None,
            };

            // Check for * or +
            let at_least_one = if *i < tokens.len() {
                match &tokens[*i].kind {
                    TokenKind::Star => {
                        *i += 1;
                        false
                    }
                    TokenKind::Plus => {
                        *i += 1;
                        true
                    }
                    _ => false,
                }
            } else {
                false
            };

            (sep.map(Maybe::Some).unwrap_or(Maybe::None), at_least_one)
        } else {
            (Maybe::None, false)
        };

        Ok(RepetitionPattern {
            pattern,
            separator,
            at_least_one,
        })
    }

    /// Lex tokens within a repetition pattern
    fn lex_pattern_tokens(tokens: &[Token]) -> Result<List<QuoteToken>, QuoteError> {
        let mut result = List::new();
        let mut i = 0;

        while i < tokens.len() {
            // Check for interpolation marker
            if i + 1 < tokens.len() && matches!(tokens[i].kind, TokenKind::Hash) {
                i += 1;
                if let TokenKind::Ident(ref name) = tokens[i].kind {
                    result.push(QuoteToken::Interpolation(Text::from(name.as_str())));
                    i += 1;
                } else {
                    return Err(QuoteError::InvalidInterpolation(Text::from(
                        "Expected identifier after '#' in repetition",
                    )));
                }
            } else {
                result.push(QuoteToken::Token(tokens[i].clone()));
                i += 1;
            }
        }

        Ok(result)
    }

    /// Expand the quote with the given context
    ///
    /// This substitutes all interpolation variables with their values
    /// from the context and expands repetition patterns.
    pub fn expand(self, context: &MetaContext) -> Result<TokenStream, QuoteError> {
        let mut stream = TokenStream::new();

        for token in self.tokens.iter() {
            Self::expand_token(token, context, &mut stream)?;
        }

        Ok(stream)
    }

    /// Expand a single quote token
    fn expand_token(
        token: &QuoteToken,
        context: &MetaContext,
        stream: &mut TokenStream,
    ) -> Result<(), QuoteError> {
        match token {
            QuoteToken::Token(t) => {
                stream.push(t.clone());
            }
            QuoteToken::Interpolation(name) => {
                if let Maybe::Some(value) = context.get_single(name.as_str()) {
                    stream.extend(value.clone());
                } else {
                    return Err(QuoteError::UnboundVariable(name.clone()));
                }
            }
            QuoteToken::Repetition {
                pattern,
                separator,
                at_least_one,
            } => {
                // Find all interpolation variables in the pattern
                let vars = Self::find_interpolations(pattern);

                if vars.is_empty() {
                    return Err(QuoteError::RepetitionMismatch(Text::from(
                        "Repetition pattern contains no interpolations",
                    )));
                }

                // Get the first variable's values to determine repetition count
                let first_var = &vars[0];
                let values = match context.get_repeat(first_var.as_str()) {
                    Maybe::Some(v) => v,
                    Maybe::None => return Err(QuoteError::UnboundVariable(first_var.clone())),
                };

                if *at_least_one && values.is_empty() {
                    return Err(QuoteError::RepetitionMismatch(Text::from(
                        "At-least-one repetition (+) requires non-empty list",
                    )));
                }

                // Verify all variables have the same length
                for var in vars.iter().skip(1) {
                    let var_values = match context.get_repeat(var.as_str()) {
                        Maybe::Some(v) => v,
                        Maybe::None => return Err(QuoteError::UnboundVariable(var.clone())),
                    };
                    if var_values.len() != values.len() {
                        return Err(QuoteError::RepetitionMismatch(Text::from(
                            "All repeated variables must have the same length",
                        )));
                    }
                }

                // Expand the pattern for each repetition
                for (idx, _) in values.iter().enumerate() {
                    if idx > 0 {
                        if let Maybe::Some(sep) = separator {
                            stream.push(Token::new(sep.clone(), Span::default()));
                        }
                    }

                    // Create a temporary context for this repetition
                    let mut rep_context = MetaContext::new();
                    for var in &vars {
                        if let Maybe::Some(var_values) = context.get_repeat(var.as_str()) {
                            rep_context.bind_single(var.clone(), var_values[idx].clone());
                        }
                    }

                    // Expand the pattern with the repetition context
                    for pat_token in pattern.iter() {
                        Self::expand_token(pat_token, &rep_context, stream)?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Find all interpolation variable names in a pattern
    fn find_interpolations(pattern: &List<QuoteToken>) -> Vec<Text> {
        let mut result = Vec::new();
        for token in pattern.iter() {
            match token {
                QuoteToken::Interpolation(name) => {
                    if !result.iter().any(|n: &Text| n.as_str() == name.as_str()) {
                        result.push(name.clone());
                    }
                }
                QuoteToken::Repetition { pattern, .. } => {
                    result.extend(Self::find_interpolations(pattern));
                }
                _ => {}
            }
        }
        result
    }

    /// Get all interpolations in this quote
    pub fn interpolations(&self) -> Map<Text, InterpolationKind> {
        let mut result = Map::new();
        for token in self.tokens.iter() {
            Self::collect_interpolations(token, &mut result);
        }
        result
    }

    fn collect_interpolations(token: &QuoteToken, result: &mut Map<Text, InterpolationKind>) {
        match token {
            QuoteToken::Interpolation(name) => {
                result.insert(name.clone(), InterpolationKind::Single(name.clone()));
            }
            QuoteToken::Repetition {
                pattern, separator, ..
            } => {
                for pat_token in pattern.iter() {
                    if let QuoteToken::Interpolation(name) = pat_token {
                        result.insert(
                            name.clone(),
                            InterpolationKind::Repeat {
                                var: name.clone(),
                                separator: separator.clone(),
                            },
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// Internal helper for repetition patterns
struct RepetitionPattern {
    pattern: List<QuoteToken>,
    separator: Maybe<TokenKind>,
    at_least_one: bool,
}

// ============================================================================
// Code generation helpers for common patterns
// ============================================================================

/// Generate an implement block
///
/// # Example
/// ```ignore
/// let impl_block = generate_impl(
///     "Debug",
///     "MyType",
///     body_stream,
///     Span::default(),
/// );
/// ```
pub fn generate_impl(
    protocol: &str,
    type_name: &str,
    body: TokenStream,
    span: Span,
) -> TokenStream {
    QuoteBuilder::with_span(span)
        .keyword("implement")
        .ident(protocol)
        .keyword("for")
        .ident(type_name)
        .punct("{")
        .interpolate(body)
        .punct("}")
        .build()
}

/// Generate a function declaration
pub fn generate_fn(
    name: &str,
    params: &[(String, String)], // (name, type)
    return_type: Option<&str>,
    body: TokenStream,
    span: Span,
) -> TokenStream {
    let mut builder = QuoteBuilder::with_span(span)
        .keyword("fn")
        .ident(name)
        .punct("(");

    // Add parameters
    for (i, (param_name, param_type)) in params.iter().enumerate() {
        if i > 0 {
            builder = builder.punct(",");
        }
        builder = builder.ident(param_name).punct(":").ident(param_type);
    }

    builder = builder.punct(")");

    // Add return type if present
    if let Some(ret) = return_type {
        builder = builder.punct("->").ident(ret);
    }

    builder.punct("{").interpolate(body).punct("}").build()
}

/// Generate a method call expression
pub fn generate_method_call(
    receiver: TokenStream,
    method: &str,
    args: Vec<TokenStream>,
    span: Span,
) -> TokenStream {
    let mut builder = QuoteBuilder::with_span(span)
        .interpolate(receiver)
        .punct(".")
        .ident(method)
        .punct("(");

    for (i, arg) in args.into_iter().enumerate() {
        if i > 0 {
            builder = builder.punct(",");
        }
        builder = builder.interpolate(arg);
    }

    builder.punct(")").build()
}

/// Generate a field access expression
pub fn generate_field_access(receiver: TokenStream, field: &str, span: Span) -> TokenStream {
    QuoteBuilder::with_span(span)
        .interpolate(receiver)
        .punct(".")
        .ident(field)
        .build()
}

/// Generate self.field access
pub fn generate_self_field(field: &str, span: Span) -> TokenStream {
    QuoteBuilder::with_span(span)
        .keyword("self")
        .punct(".")
        .ident(field)
        .build()
}

/// Generate a struct literal
pub fn generate_struct_literal(
    type_name: &str,
    fields: &[(String, TokenStream)],
    span: Span,
) -> TokenStream {
    let mut builder = QuoteBuilder::with_span(span).ident(type_name).punct("{");

    for (i, (field_name, field_value)) in fields.iter().enumerate() {
        if i > 0 {
            builder = builder.punct(",");
        }
        builder = builder
            .ident(field_name)
            .punct(":")
            .interpolate(field_value.clone());
    }

    builder.punct("}").build()
}

/// Generate a match arm
pub fn generate_match_arm(pattern: TokenStream, body: TokenStream, span: Span) -> TokenStream {
    QuoteBuilder::with_span(span)
        .interpolate(pattern)
        .punct("=>")
        .interpolate(body)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hygiene::HygieneContext;

    #[test]
    fn test_quote_builder_basic() {
        let stream = QuoteBuilder::new()
            .keyword("let")
            .ident("x")
            .punct("=")
            .int(42)
            .punct(";")
            .build();

        assert_eq!(stream.len(), 5);
    }

    #[test]
    fn test_quote_builder_hygienic() {
        let builder = QuoteBuilder::new();
        let stream1 = builder.hygienic_ident("temp").build();

        let builder2 = QuoteBuilder::new();
        let stream2 = builder2.hygienic_ident("temp").build();

        // Both should generate different identifiers
        assert_eq!(stream1.len(), 1);
        assert_eq!(stream2.len(), 1);
    }

    #[test]
    fn test_quote_builder_repeat() {
        let items = vec!["a", "b", "c"];
        let stream = QuoteBuilder::new()
            .repeat(items, Some(","), |item| ident(item, Span::default()))
            .build();

        // Should have: a , b , c = 5 tokens
        assert_eq!(stream.len(), 5);
    }

    #[test]
    fn test_generate_impl() {
        let body = QuoteBuilder::new()
            .keyword("fn")
            .ident("fmt")
            .punct("(")
            .punct(")")
            .punct("{")
            .punct("}")
            .build();

        let impl_block = generate_impl("Debug", "MyType", body, Span::default());
        assert!(!impl_block.is_empty());
    }

    #[test]
    fn test_format_ident() {
        let stream = format_ident("get_{}", &["name"], Span::default());
        assert_eq!(stream.len(), 1);
    }

    #[test]
    fn test_concat() {
        let stream = concat(&["hello", " ", "world"], Span::default());
        assert_eq!(stream.len(), 1);
    }

    #[test]
    fn test_quote_parse_simple() {
        let quote = Quote::parse("let x = 42;").unwrap();
        assert!(!quote.tokens.is_empty());
    }

    #[test]
    fn test_quote_single_interpolation() {
        let quote = Quote::parse("let # x = # value;").unwrap();
        let interpolations = quote.interpolations();
        assert!(interpolations.contains_key(&Text::from("x")));
        assert!(interpolations.contains_key(&Text::from("value")));
    }

    #[test]
    fn test_quote_expand_single() {
        let quote = Quote::parse("let # name = 42;").unwrap();
        let mut context = MetaContext::new();
        context.bind_single(Text::from("name"), ident("my_var", Span::default()));

        let result = quote.expand(&context).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_quote_repetition() {
        let quote = Quote::parse("# ( # item ) , *").unwrap();
        let mut context = MetaContext::new();
        let items = List::from_iter(vec![
            ident("a", Span::default()),
            ident("b", Span::default()),
            ident("c", Span::default()),
        ]);
        context.bind_repeat(Text::from("item"), items);

        let result = quote.expand(&context).unwrap();
        // Should have: a , b , c (possibly with trailing separator or star) = 5-6 tokens
        assert!(result.len() >= 5 && result.len() <= 6);
    }

    #[test]
    fn test_hygiene_context() {
        let ctx = HygieneContext::new();
        let id1 = ctx.generate("temp");
        let id2 = ctx.generate("temp");

        // Generated IDs should be different
        assert_ne!(id1.as_str(), id2.as_str());
        assert!(HygieneContext::is_hygienic(id1.as_str()));
        assert!(HygieneContext::is_hygienic(id2.as_str()));
    }

    #[test]
    fn test_hygiene_base_name() {
        let ctx = HygieneContext::new();
        let id = ctx.generate("my_var");
        let base = HygieneContext::base_name(id.as_str());
        assert_eq!(base.as_str(), "my_var");
    }

    #[test]
    fn test_meta_context_bindings() {
        let mut ctx = MetaContext::new();
        ctx.bind_single(Text::from("x"), ident("foo", Span::default()));

        assert!(ctx.get_single("x").is_some());
        assert!(ctx.get_single("y").is_none());
    }

    #[test]
    fn test_quote_unbound_variable() {
        let quote = Quote::parse("let # name = 42;").unwrap();
        let context = MetaContext::new(); // Empty context

        let result = quote.expand(&context);
        assert!(result.is_err());
        match result {
            Err(QuoteError::UnboundVariable(var)) => {
                assert_eq!(var.as_str(), "name");
            }
            _ => panic!("Expected UnboundVariable error"),
        }
    }

    #[test]
    fn test_quote_repetition_mismatch() {
        let quote = Quote::parse("# ( # a # b ) , *").unwrap();
        let mut context = MetaContext::new();
        context.bind_repeat(
            Text::from("a"),
            List::from_iter(vec![ident("x", Span::default())]),
        );
        context.bind_repeat(
            Text::from("b"),
            List::from_iter(vec![
                ident("y", Span::default()),
                ident("z", Span::default()),
            ]),
        );

        let result = quote.expand(&context);
        assert!(result.is_err());
    }

    #[test]
    fn test_to_tokens_expr_binary() {
        use verum_ast::BinOp;
        use verum_ast::expr::{Expr, ExprKind};
        use verum_common::Heap;

        let left = Heap::new(Expr::new(
            ExprKind::Path(verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                "a",
                Span::default(),
            ))),
            Span::default(),
        ));
        let right = Heap::new(Expr::new(
            ExprKind::Path(verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                "b",
                Span::default(),
            ))),
            Span::default(),
        ));
        let expr = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left,
                right,
            },
            Span::default(),
        );

        let stream = expr.into_token_stream();
        assert!(stream.len() >= 3); // a + b
    }

    #[test]
    fn test_to_tokens_pattern_wildcard() {
        use verum_ast::pattern::{Pattern, PatternKind};

        let pattern = Pattern::new(PatternKind::Wildcard, Span::default());
        let stream = pattern.into_token_stream();
        assert_eq!(stream.len(), 1);
    }

    #[test]
    fn test_to_tokens_type_reference() {
        use verum_ast::ty::{Type, TypeKind};
        use verum_common::Heap;

        let inner = Heap::new(Type::new(
            TypeKind::Path(verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                "Int",
                Span::default(),
            ))),
            Span::default(),
        ));
        let ref_type = Type::new(
            TypeKind::Reference {
                mutable: false,
                inner,
            },
            Span::default(),
        );

        let stream = ref_type.into_token_stream();
        assert!(stream.len() >= 2); // & Int
    }
}
