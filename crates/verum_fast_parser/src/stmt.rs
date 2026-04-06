//! Statement parser for Verum using hand-written recursive descent.
//!
//! This module implements parsing for all statement forms:
//! - Let bindings: `let x = expr`, `let mut x: Type = expr`
//! - Let-else: `let Some(x) = opt else { return }`
//! - Expression statements: `expr;` or `expr` (tail position)
//! - Defer statements: `defer { cleanup() }`
//! - Provide statements: `provide ContextName = expr;`
//! - Empty statements: `;`

use verum_ast::{Block, Expr, ExprKind, Pattern, Span, Stmt, StmtKind, Type};
use verum_common::{Heap, List, Maybe, Text};
use verum_lexer::{Token, TokenKind};

use crate::error::{ParseError, ParseErrorKind};
use crate::parser::{ParseResult, RecursiveParser};

// ============================================================================
// Hand-written Recursive Descent API
// ============================================================================

impl<'a> RecursiveParser<'a> {
    /// Parse a single statement.
    ///
    /// Statements include:
    /// - Let bindings (including let-else)
    /// - Expression statements
    /// - Defer statements
    /// - Provide statements
    /// - Empty statements (just semicolon)
    pub fn parse_stmt(&mut self) -> ParseResult<Stmt> {
        let start_pos = self.stream.position();

        // E048: Empty statement (standalone semicolon) is not allowed
        if self.stream.check(&TokenKind::Semicolon) {
            let span = self.stream.current_span();
            return Err(ParseError::invalid_syntax(
                "empty statement (standalone `;`) is not allowed; remove the extra semicolon",
                span,
            ));
        }

        // Special case: if we see @ followed by identifier/keyword and it's NOT followed
        // by a declaration keyword, treat it as an expression statement (macro call)
        // This handles: @log_debug("msg"); vs @derive(Clone) type Foo = ...;
        if self.stream.check(&TokenKind::At) {
            // Peek ahead to see what follows the @ construct
            if self.looks_like_macro_call_not_attribute() {
                // This is a macro call expression statement, not an attribute
                return self.parse_expr_stmt();
            }
        }

        // Parse attributes before statements (e.g., @unroll for i in ..., @likely if ...)
        // Attributes like @unroll, @likely, @cold can precede statements and expressions
        let attributes = self.parse_attributes()?;

        // Try to parse different statement types
        // Order matters: let-else must be tried before let
        let mut stmt = if self.stream.check(&TokenKind::Let) {
            // Try let-else first (requires lookahead)
            if self.looks_like_let_else() {
                self.parse_let_else_stmt()?
            } else {
                self.parse_let_stmt()?
            }
        } else if self.stream.check(&TokenKind::Defer) {
            self.parse_defer_stmt()?
        } else if self.stream.check(&TokenKind::Errdefer) {
            self.parse_errdefer_stmt()?
        } else if self.stream.check(&TokenKind::Provide) {
            self.parse_provide_stmt()?
        } else if self.stream.check(&TokenKind::Type)
            || self.stream.check(&TokenKind::Fn)
            || self.stream.check(&TokenKind::Const)
            || self.stream.check(&TokenKind::Mount)
            || self.stream.check(&TokenKind::Static)
            || self.stream.check(&TokenKind::Implement)
            || self.stream.check(&TokenKind::Context)
            // Check for "async fn|context" vs "async { }" - only treat as item if followed by fn/context
            || (self.stream.check(&TokenKind::Async) && matches!(
                self.stream.peek_nth_kind(1),
                Some(TokenKind::Fn) | Some(TokenKind::Context)
            ))
            || self.stream.check(&TokenKind::Pure)   // pure fn, pure async fn
            // Check for "meta fn" vs "meta { }" - only treat as item if followed by fn/async
            || (self.stream.check(&TokenKind::Meta) && matches!(
                self.stream.peek_nth_kind(1),
                Some(TokenKind::Fn) | Some(TokenKind::Async)
            ))
            || self.stream.check(&TokenKind::Extern) // extern fn
            // Check for "unsafe fn" vs "unsafe { }" - only treat as item if followed by fn
            || (self.stream.check(&TokenKind::Unsafe) && self.stream.peek_nth_kind(1) == Some(&TokenKind::Fn))
        {
            // Local item declaration: type, fn, const, static, implement, import,
            // or function with modifiers (async, pure, meta, unsafe, extern) inside a block
            let mut item = self.parse_item()?;
            // Merge pre-parsed attributes into the item. parse_item() parses its own
            // attributes from the token stream, but when @attr is followed by `fn`,
            // parse_stmt consumes the attribute first. Prepend those to the item.
            if !attributes.is_empty() {
                let mut merged = List::from(attributes.clone());
                merged.extend(item.attributes.iter().cloned());
                item.attributes = merged;
                // Also propagate to the inner FunctionDecl if present
                if let verum_ast::ItemKind::Function(ref mut func) = item.kind {
                    let mut func_merged = List::new();
                    for attr in &attributes {
                        func_merged.push(attr.clone());
                    }
                    func_merged.extend(func.attributes.iter().cloned());
                    func.attributes = func_merged;
                }
            }
            Stmt::new(StmtKind::Item(item.clone()), item.span)
        } else if self.stream.check(&TokenKind::Invariant)
            || self.stream.check(&TokenKind::Decreases)
        {
            // Loop annotation statements: `invariant expr;` or `decreases expr;`
            // These can appear inside loop bodies for verification annotations.
            // Parse the keyword and expression, then wrap as an expression statement.
            self.stream.advance(); // consume invariant/decreases keyword
            let expr = self.parse_expr_no_struct()?;
            let has_semi = self.stream.consume(&TokenKind::Semicolon).is_some();
            let span = self.stream.make_span(start_pos);
            // Treat as an expression statement - the verifier will pick up the
            // invariant/decreases semantics from the surrounding loop context
            Stmt::new(StmtKind::Expr { expr, has_semi }, span)
        } else if self.stream.check(&TokenKind::For)
            || self.stream.check(&TokenKind::While)
            || self.stream.check(&TokenKind::Loop)
        {
            // Block-form control flow statements (for, while, loop).
            // These end at their closing brace and should NOT allow binary operators
            // to follow without explicit grouping. This prevents:
            //   for i in 0..n { body }
            //   *ptr = value;
            // from being parsed as: (for_loop) * (ptr = value)
            //
            // Note: `if`, `match`, and `unsafe` are excluded because they return values and
            // are commonly used in expressions like: let x = if cond { a } else { b };
            // or: unsafe { func() } >= threshold
            let expr = self.parse_prefix_expr()?;
            let span = expr.span;
            let has_semi = self.stream.consume(&TokenKind::Semicolon).is_some();
            Stmt::new(StmtKind::Expr { expr, has_semi }, span)
        } else if let Some(TokenKind::Ident(name)) = self.stream.peek_kind() {
            if name.as_str() == "ghost" {
                // Ghost blocks and ghost statements: ghost { ... }, ghost invariant { ... }
                // These are verification-only constructs, erased at runtime
                match self.stream.peek_nth_kind(1) {
                    Some(TokenKind::LBrace) => {
                        // ghost { ... } - ghost block
                        self.stream.advance(); // consume `ghost`
                        let block = self.parse_block()?;
                        let span = self.stream.make_span(start_pos);
                        Stmt::new(StmtKind::Expr { expr: Expr::new(ExprKind::Block(block), span), has_semi: false }, span)
                    }
                    Some(TokenKind::Invariant) => {
                        // ghost invariant { ... }
                        self.stream.advance(); // consume `ghost`
                        self.stream.advance(); // consume `invariant`
                        let expr = if self.stream.check(&TokenKind::LBrace) {
                            let block = self.parse_block()?;
                            let span = self.stream.make_span(start_pos);
                            Expr::new(ExprKind::Block(block), span)
                        } else {
                            self.parse_expr_no_struct()?
                        };
                        let has_semi = self.stream.consume(&TokenKind::Semicolon).is_some();
                        let span = self.stream.make_span(start_pos);
                        Stmt::new(StmtKind::Expr { expr, has_semi }, span)
                    }
                    _ => {
                        // Expression statement starting with ghost identifier
                        self.parse_expr_stmt()?
                    }
                }
            } else {
                // Expression statement
                self.parse_expr_stmt()?
            }
        } else {
            // Expression statement
            self.parse_expr_stmt()?
        };

        // Attach attributes to the statement if any were parsed
        if !attributes.is_empty() {
            stmt.attributes = attributes;
        }

        Ok(stmt)
    }

    /// Parse a block: `{ stmt1; stmt2; expr }`
    ///
    /// A block consists of zero or more statements followed by an optional
    /// trailing expression (no semicolon).
    ///
    /// Parse a block `{ ... }`.
    ///
    /// Recursion depth is checked to prevent stack overflow from
    /// deeply nested blocks like `{ { { { ... } } } }`.
    pub fn parse_block(&mut self) -> ParseResult<Block> {
        self.enter_recursion()?;
        let result = self.parse_block_inner();
        self.exit_recursion();
        result
    }

    /// Inner implementation of parse_block
    fn parse_block_inner(&mut self) -> ParseResult<Block> {
        let start_token = self.stream.expect(TokenKind::LBrace)?;
        let start_span = start_token.span;

        // Handle empty block: { }
        if self.stream.check(&TokenKind::RBrace) {
            let end_token = self.stream.expect(TokenKind::RBrace)?;
            let span = Span::new(start_span.start, end_token.span.end, start_span.file_id);
            return Ok(Block::new(List::new(), Maybe::None, span));
        }

        let mut stmts = Vec::new();
        let mut expr = Maybe::None;

        // Parse statements until we hit a closing brace or EOF
        while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }

            let pos_before = self.stream.position();

            // Try to parse a statement
            // Note: E048 (empty statement) is checked inside parse_stmt
            let stmt_result = self.parse_stmt();

            match stmt_result {
                Ok(stmt) => {
                    // Check if this is an expression statement without semicolon
                    // If so, it might be the trailing expression
                    let is_expr_no_semi = matches!(
                        stmt.kind,
                        StmtKind::Expr {
                            has_semi: false,
                            ..
                        }
                    );
                    let next_is_rbrace = self.stream.check(&TokenKind::RBrace);

                    // Don't convert to trailing expression if statement has attributes.
                    // Attributes (like @cfg for conditional compilation) must be preserved
                    // on statements for filtering to work. If we extract just the expression,
                    // the attributes would be lost.
                    // Example: `@cfg(runtime = "embedded") { call() }` at end of block
                    // must stay as a statement so @cfg filtering applies.
                    let has_attributes = !stmt.attributes.is_empty();

                    if is_expr_no_semi && next_is_rbrace && !has_attributes {
                        if let StmtKind::Expr { expr: e, .. } = stmt.kind {
                            expr = Maybe::Some(Heap::new(e));
                        }
                        break;
                    }
                    // Only consume optional trailing semicolon for block-form statements
                    // that don't inherently end with a semicolon:
                    // - defer { }     - ends with }, optional trailing ;
                    // - errdefer { }  - ends with }, optional trailing ;
                    // - let-else { }  - ends with }, optional trailing ;
                    // - provide ... in { } - ends with }, optional trailing ;
                    // DO NOT consume for let/expr statements - they already handle their own semicolons
                    // and consuming here would hide double-semicolon errors (E048 for `;;`)
                    let is_block_form = matches!(
                        &stmt.kind,
                        StmtKind::Defer(_)
                            | StmtKind::Errdefer(_)
                            | StmtKind::LetElse { .. }
                            | StmtKind::ProvideScope { .. }
                    );
                    if is_block_form && self.stream.check(&TokenKind::Semicolon) {
                        self.stream.advance();
                    }

                    stmts.push(stmt);
                }
                Err(e) => {
                    self.error(e);
                    let skipped = self.synchronize();
                    // If no progress was made, skip at least one token to avoid infinite loop
                    if skipped == 0 && self.stream.position() == pos_before {
                        self.stream.advance();
                    }
                }
            }
        }

        // E011: Check for unclosed block
        if !self.stream.check(&TokenKind::RBrace) {
            return Err(ParseError::stmt_unclosed_block(
                self.stream.make_span(start_span.start as usize),
            ).with_help("missing closing '}' for block"));
        }
        // SAFETY: check() above confirmed RBrace exists, so advance() succeeds
        let end_token = self.stream.advance()
            .expect("RBrace check above guarantees token exists"); // consume }
        let end_span = end_token.span;
        let span = Span::new(start_span.start, end_span.end, start_span.file_id);

        Ok(Block::new(List::from(stmts), expr, span))
    }

    /// Parse a let statement: `let pattern = expr;` or `let pattern: Type = expr;`
    fn parse_let_stmt(&mut self) -> ParseResult<Stmt> {
        let start_token = self.stream.expect(TokenKind::Let)?;
        let start_span = start_token.span;

        // E040: Check for missing pattern (let = 5;)
        if self.stream.check(&TokenKind::Eq) {
            return Err(ParseError::let_missing_pattern(self.stream.current_span()));
        }

        // Literal patterns (like `let 0 = x;`) and range patterns (like `let 0..10 = x;`)
        // are valid in Verum - they're used for refutable pattern matching.
        // The pattern parser handles these correctly.

        // Parse pattern using hand-written parser
        let pattern = self.parse_pattern_adapter()?;

        // E088: Check for guard in let (guards only in match)
        if self.stream.check(&TokenKind::If) {
            return Err(ParseError::pattern_invalid_let(
                "guards are not allowed in let patterns (only in match)",
                self.stream.current_span(),
            ));
        }

        // E088: Check for or-pattern in let without =
        if self.stream.check(&TokenKind::Pipe) {
            return Err(ParseError::pattern_invalid_let(
                "or-patterns are not allowed in let statements",
                self.stream.current_span(),
            ));
        }

        // Optional type annotation
        let ty = if self.stream.consume(&TokenKind::Colon).is_some() {
            // E043: Check for missing type after colon (let x: = 5;)
            if self.stream.check(&TokenKind::Eq) {
                return Err(ParseError::let_invalid_type_or_pattern(
                    "missing type after ':'",
                    self.stream.current_span(),
                ));
            }
            // E043: Check for invalid type (let x: 123 = 5;)
            if matches!(self.stream.peek_kind(), Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_))) {
                return Err(ParseError::let_invalid_type_or_pattern(
                    "literals cannot be used as types",
                    self.stream.current_span(),
                ));
            }
            let ty = self.parse_type_adapter()?;
            Maybe::Some(ty)
        } else {
            Maybe::None
        };

        // E088: Check for invalid assignment operators
        if self.stream.check(&TokenKind::EqEq) {
            return Err(ParseError::pattern_invalid_let(
                "use '=' not '==' for assignment in let",
                self.stream.current_span(),
            ));
        }
        if self.stream.check(&TokenKind::FatArrow) {
            return Err(ParseError::pattern_invalid_let(
                "use '=' not '=>' for assignment in let",
                self.stream.current_span(),
            ));
        }

        // E042: Check for missing equals when there's something that looks like a value
        // This catches cases like `let x 5;` (missing =)
        if !self.stream.check(&TokenKind::Eq)
            && !self.stream.check(&TokenKind::Semicolon)
            && !self.stream.check(&TokenKind::RBrace)
            && !self.stream.at_end()
            && ty.is_none()
        {
            // Check for literals or identifiers that look like values
            if matches!(
                self.stream.peek_kind(),
                Some(TokenKind::Integer(_))
                    | Some(TokenKind::Float(_))
                    | Some(TokenKind::InterpolatedString(_))
                    | Some(TokenKind::Char(_))
                    | Some(TokenKind::True)
                    | Some(TokenKind::False)
            ) {
                return Err(ParseError::let_missing_equals(self.stream.current_span()));
            }
            // Also catch operators after pattern (expressions cannot be patterns)
            if matches!(
                self.stream.peek_kind(),
                Some(TokenKind::Plus)
                    | Some(TokenKind::Minus)
                    | Some(TokenKind::Star)
                    | Some(TokenKind::Slash)
            ) {
                return Err(ParseError::pattern_invalid_let(
                    "expressions cannot be used as patterns",
                    self.stream.current_span(),
                ));
            }
        }

        // Optional initializer
        let value = if self.stream.consume(&TokenKind::Eq).is_some() {
            // E041: Check for missing value after equals (let x =;)
            if self.stream.check(&TokenKind::Semicolon) {
                return Err(ParseError::let_missing_value(self.stream.current_span()));
            }
            let expr = self.parse_expr_adapter()?;
            Maybe::Some(expr)
        } else {
            Maybe::None
        };

        // Semicolon is required unless at block end or EOF
        let end_span = if self.stream.check(&TokenKind::RBrace) || self.stream.at_end() {
            // At block end or EOF - semicolon optional
            if let Some(semi) = self.stream.consume(&TokenKind::Semicolon) {
                semi.span
            } else {
                // Use the span of the last parsed element
                value
                    .as_ref()
                    .map(|v| v.span)
                    .unwrap_or_else(|| ty.as_ref().map(|t| t.span).unwrap_or(pattern.span))
            }
        } else if self.stream.check(&TokenKind::Semicolon) {
            // Semicolon present
            // SAFETY: check() above confirmed Semicolon exists
            self.stream.advance()
                .expect("Semicolon check above guarantees token exists").span
        } else {
            // E010: Missing semicolon - another statement/expression follows
            return Err(ParseError::missing_semicolon(self.stream.current_span(), "let statement"));
        };

        let span = Span::new(start_span.start, end_span.end, start_span.file_id);

        Ok(Stmt::new(StmtKind::Let { pattern, ty, value }, span))
    }

    /// Parse a let-else statement: `let pattern = expr else { block };`
    fn parse_let_else_stmt(&mut self) -> ParseResult<Stmt> {
        let start_token = self.stream.expect(TokenKind::Let)?;
        let start_span = start_token.span;

        // Parse pattern
        let pattern = self.parse_pattern_adapter()?;

        // Optional type annotation
        let ty = if self.stream.consume(&TokenKind::Colon).is_some() {
            let ty = self.parse_type_adapter()?;
            Maybe::Some(ty)
        } else {
            Maybe::None
        };

        // Required initializer
        self.stream.expect(TokenKind::Eq)?;
        let value = self.parse_expr_adapter()?;

        // Else keyword and block
        self.stream.expect(TokenKind::Else)?;

        // E040: Check for missing else block (let Some(x) = value else;)
        if self.stream.check(&TokenKind::Semicolon) || self.stream.at_end() {
            return Err(ParseError::let_missing_pattern(self.stream.current_span())
                .with_help("let-else requires a block after 'else'"));
        }

        let else_block = self.parse_block()?;

        // Optional semicolon
        let end_span = if let Some(semi) = self.stream.consume(&TokenKind::Semicolon) {
            semi.span
        } else {
            else_block.span
        };

        let span = Span::new(start_span.start, end_span.end, start_span.file_id);

        Ok(Stmt::new(
            StmtKind::LetElse {
                pattern,
                ty,
                value,
                else_block,
            },
            span,
        ))
    }

    /// Parse a defer statement: `defer expr;` or `defer { block }`
    fn parse_defer_stmt(&mut self) -> ParseResult<Stmt> {
        // Consume "defer" keyword
        let start_token = self.stream.expect(TokenKind::Defer)?;
        let start_span = start_token.span;

        // E045: Check for missing defer body (defer;)
        if self.stream.check(&TokenKind::Semicolon) {
            return Err(ParseError::defer_invalid(
                "defer statement requires an expression or block",
                self.stream.current_span(),
            ));
        }

        // E045: Check for invalid defer expression (defer let x = 5;)
        if self.stream.check(&TokenKind::Let) {
            return Err(ParseError::defer_invalid(
                "defer requires an expression, not a let statement",
                self.stream.current_span(),
            ));
        }

        // Parse the deferred expression (could be a block or a simple expr)
        let expr = self.parse_expr_adapter()?;
        let expr_span = expr.span;

        // Block expressions don't require semicolon, others do
        let is_block = matches!(expr.kind, verum_ast::ExprKind::Block(_));

        let end_span = if is_block {
            // Optional semicolon for block expressions
            if let Some(semi) = self.stream.consume(&TokenKind::Semicolon) {
                semi.span
            } else {
                expr_span
            }
        } else if self.allows_semicolon_omission() {
            // Optional semicolon for non-block expressions
            if let Some(semi) = self.stream.consume(&TokenKind::Semicolon) {
                semi.span
            } else {
                expr_span
            }
        } else {
            // Required semicolon
            let semi = self.stream.expect(TokenKind::Semicolon)?;
            semi.span
        };

        let span = Span::new(start_span.start, end_span.end, start_span.file_id);

        Ok(Stmt::new(StmtKind::Defer(expr), span))
    }

    /// Parse an errdefer statement: `errdefer expr;` or `errdefer { block }`
    ///
    /// Errdefer is similar to defer but only executes when the scope exits
    /// via an error path (e.g., when a function returns an error or panics).
    ///
    /// Grammar from verum.ebnf v2.8:
    /// ```text
    /// defer_stmt = 'defer' , defer_body | 'errdefer' , defer_body ;
    /// defer_body = expression , ';' | block_expr ;
    /// ```
    ///
    /// Examples:
    /// - `errdefer file.close();` - expression form
    /// - `errdefer { cleanup(); log_error(); }` - block form
    fn parse_errdefer_stmt(&mut self) -> ParseResult<Stmt> {
        // Consume "errdefer" keyword
        let start_token = self.stream.expect(TokenKind::Errdefer)?;
        let start_span = start_token.span;

        // E045: Check for missing errdefer body (errdefer;)
        if self.stream.check(&TokenKind::Semicolon) {
            return Err(ParseError::defer_invalid(
                "errdefer statement requires an expression or block",
                self.stream.current_span(),
            ));
        }

        // E045: Check for invalid errdefer expression (errdefer let x = 5;)
        if self.stream.check(&TokenKind::Let) {
            return Err(ParseError::defer_invalid(
                "errdefer requires an expression, not a let statement",
                self.stream.current_span(),
            ));
        }

        // Parse the deferred expression (could be a block or a simple expr)
        let expr = self.parse_expr_adapter()?;
        let expr_span = expr.span;

        // Block expressions don't require semicolon, others do
        let is_block = matches!(expr.kind, verum_ast::ExprKind::Block(_));

        let end_span = if is_block {
            // Optional semicolon for block expressions
            if let Some(semi) = self.stream.consume(&TokenKind::Semicolon) {
                semi.span
            } else {
                expr_span
            }
        } else if self.allows_semicolon_omission() {
            // Optional semicolon for non-block expressions
            if let Some(semi) = self.stream.consume(&TokenKind::Semicolon) {
                semi.span
            } else {
                expr_span
            }
        } else {
            // Required semicolon
            let semi = self.stream.expect(TokenKind::Semicolon)?;
            semi.span
        };

        let span = Span::new(start_span.start, end_span.end, start_span.file_id);

        Ok(Stmt::new(StmtKind::Errdefer(expr), span))
    }

    /// Parse a provide statement: `provide ContextName = expr;`
    /// or block-scoped provide: `provide ContextName = expr in { block }`
    ///
    /// Supports both simple context names and path-based contexts:
    /// - Simple: `provide Logger = logger_impl;`
    /// - Path-based: `provide FileSystem.Write = write_impl;`
    /// - With alias: `provide Database as source = source_db;`
    /// - Block-scoped: `provide Database = db in { query_users() }`
    /// - Block-scoped with alias: `provide Database as backup = db in { query_users() }`
    ///
    /// Grammar (context provide with aliases and sub-context paths):
    /// ```text
    /// provide_stmt = 'provide' , context_path , [ 'as' , identifier ] , '=' , expression , ( ';' | 'in' , block_expr ) ;
    /// context_path = identifier , { '.' , identifier } ;
    /// ```
    fn parse_provide_stmt(&mut self) -> ParseResult<Stmt> {
        // Consume "provide" keyword. Supports:
        // - Single: `provide Ctx = value { block }`
        // - Multi:  `provide A = x, B = y { block }` (desugars to nested)
        // - With 'in': `provide Ctx = value in { block }`
        let start_token = self.stream.expect(TokenKind::Provide)?;
        let start_span = start_token.span;

        // E044: Check for missing provide context (provide;)
        if self.stream.check(&TokenKind::Semicolon) {
            return Err(ParseError::provide_invalid(
                "provide statement requires a context name",
                self.stream.current_span(),
            ));
        }

        // E044: Check for invalid context (provide 123 = value;)
        if matches!(self.stream.peek_kind(), Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_))) {
            return Err(ParseError::provide_invalid(
                "provide requires an identifier as context name, not a literal",
                self.stream.current_span(),
            ));
        }

        // Parse context name (identifier or path like FileSystem.Write)
        // Supports generic args: Repository<Order>, Map<Text, Int>
        // For now, we store this as a single Text string with dots and generic args
        let mut context = self.consume_ident()?;

        // Support path-based contexts: Context.SubContext.Field
        while self.stream.consume(&TokenKind::Dot).is_some() {
            let segment = self.consume_ident()?;
            context = Text::from(format!("{}.{}", context.as_str(), segment.as_str()));
        }

        // Support generic type args on context name: Repository<Order>
        if self.stream.check(&TokenKind::Lt) {
            // Skip the generic args - we just need them for context name matching
            let mut depth = 0;
            let mut generic_text = String::from("<");
            self.stream.advance(); // consume '<'
            depth += 1;
            while depth > 0 && !self.stream.at_end() {
                match self.stream.peek_kind() {
                    Some(TokenKind::Lt) => {
                        generic_text.push('<');
                        depth += 1;
                        self.stream.advance();
                    }
                    Some(TokenKind::Gt) | Some(TokenKind::GtGt) => {
                        if matches!(self.stream.peek_kind(), Some(TokenKind::GtGt)) {
                            generic_text.push_str(">>");
                            depth -= 2;
                        } else {
                            generic_text.push('>');
                            depth -= 1;
                        }
                        self.stream.advance();
                    }
                    Some(TokenKind::Comma) => {
                        generic_text.push_str(", ");
                        self.stream.advance();
                    }
                    Some(TokenKind::Ident(name)) => {
                        generic_text.push_str(name.as_str());
                        self.stream.advance();
                    }
                    _ => {
                        // Other tokens - just skip them
                        self.stream.advance();
                    }
                }
            }
            context = Text::from(format!("{}{}", context.as_str(), generic_text));
        }

        // Layer expansion: `provide LayerName;` (no '=' sign)
        // When a provide has no value, it's a layer expansion — VBC codegen will
        // look up the layer and expand it into individual provides.
        if self.stream.check(&TokenKind::Semicolon) {
            let semi_span = self.stream.current_span();
            self.stream.advance(); // consume semicolon
            let span = Span::new(start_span.start, semi_span.end, start_span.file_id);
            // Use empty tuple () as sentinel value for layer expansion
            let unit_value = Expr::new(
                verum_ast::ExprKind::Tuple(verum_common::List::new()),
                span,
            );
            return Ok(Stmt::new(
                StmtKind::Provide {
                    context,
                    alias: Maybe::None,
                    value: Heap::new(unit_value),
                },
                span,
            ));
        }

        // E044: Check for missing provide block (provide Database followed by EOF/newline)
        if self.stream.check(&TokenKind::RBrace) || self.stream.at_end() {
            return Err(ParseError::provide_invalid(
                "provide statement requires '=' and a value expression",
                self.stream.current_span(),
            ));
        }

        // Handle named context binding: provide name: Type = value
        // e.g., provide primary_db: Database = prod
        if self.stream.check(&TokenKind::Colon) {
            self.stream.advance(); // consume ':'
            let _ty = self.parse_type()?; // parse Type (ignored at AST level for now)
            // After type, we must have '='
        }

        // E044: Check for missing equals (provide Database db; - identifier after context without =)
        if !self.stream.check(&TokenKind::Eq) && !self.stream.check(&TokenKind::As) && !self.stream.check(&TokenKind::Dot) {
            // Not =, not 'as', not '.' - this is an error unless it's the alias or equals
            if let Some(TokenKind::Ident(_)) = self.stream.peek_kind() {
                return Err(ParseError::provide_invalid(
                    "missing '=' in provide statement",
                    self.stream.current_span(),
                ));
            }
        }

        // Parse optional alias: 'as' identifier
        // This enables multiple instances of the same context type:
        //   provide Database as source = PostgresHandler::connect(source_url);
        //   provide Database as target = PostgresHandler::connect(target_url);
        // Alias enables multiple instances of the same context type (e.g., `provide Database as source = ...`)
        let alias = if self.stream.check(&TokenKind::As) {
            self.stream.advance(); // consume 'as'
            let alias_name = self.consume_ident()?;
            Maybe::Some(alias_name)
        } else {
            Maybe::None
        };

        // E044: Check for missing equals after alias (provide Database as db;)
        if alias.is_some() && !self.stream.check(&TokenKind::Eq) {
            // After 'as alias', we must have '='
            if self.stream.check(&TokenKind::Semicolon) || self.stream.at_end() {
                return Err(ParseError::provide_invalid(
                    "missing '=' after alias in provide statement",
                    self.stream.current_span(),
                ));
            }
        }

        // Expect =
        if !self.stream.check(&TokenKind::Eq) {
            return Err(ParseError::provide_invalid(
                "expected '=' in provide statement",
                self.stream.current_span(),
            ));
        }
        self.stream.advance(); // consume =

        // Parse value expression with binding power 7 to stop before 'in' operator.
        // The 'in' operator has binding power (6, 7), so using bp=7 prevents it from
        // being consumed as a binary operator, allowing us to detect the provide scope syntax.
        //
        // Use full expression parsing (not no_struct) to handle named struct literals:
        //   provide Logger = ConsoleLogger {} in { ... }
        // If the next token after '=' is an identifier and the token after THAT is '{',
        // we need to allow struct literal parsing. But if there's no identifier before '{',
        // the '{' is a scope block.
        let value = if matches!(self.stream.peek_kind(), Some(TokenKind::Ident(_)))
            || matches!(self.stream.peek_kind(), Some(TokenKind::Some | TokenKind::None | TokenKind::Ok | TokenKind::Err))
        {
            // Identifier-led expression: allow struct literal parsing
            self.parse_expr_bp(7)?
        } else {
            // Non-identifier expression: use no_struct to prevent { from being consumed
            self.parse_expr_bp_no_struct(7)?
        };
        let value_span = value.span;

        // Handle multi-provide: provide A = x, B = y { block }
        // When we see comma after value, parse additional provide bindings.
        // These desugar into nested ProvideScope statements.
        if self.stream.check(&TokenKind::Comma) {
            // Multi-provide: collect remaining bindings, then expect block
            let mut bindings = vec![(context.clone(), alias.clone(), value)];
            while self.stream.consume(&TokenKind::Comma).is_some() {
                // Parse next binding: name[: Type] = value
                let next_context = self.consume_ident()?;
                let mut next_ctx_name = next_context.to_string();
                while self.stream.consume(&TokenKind::Dot).is_some() {
                    let seg = self.consume_ident()?;
                    next_ctx_name.push('.');
                    next_ctx_name.push_str(seg.as_str());
                }
                let next_ctx = Text::from(next_ctx_name);

                // Optional type annotation
                if self.stream.check(&TokenKind::Colon) {
                    self.stream.advance();
                    let _ty = self.parse_type()?;
                }

                // Optional alias
                let next_alias = if self.stream.check(&TokenKind::As) {
                    self.stream.advance();
                    Maybe::Some(self.consume_ident()?)
                } else {
                    Maybe::None
                };

                self.stream.expect(TokenKind::Eq)?;
                let next_value = self.parse_expr_bp_no_struct(7)?;
                bindings.push((next_ctx, next_alias, next_value));
            }

            // Expect block scope - parse the shared block
            let block = self.parse_block()?;
            let block_span = block.span;
            let block_expr = Expr::new(verum_ast::ExprKind::Block(block), block_span);

            // Build nested ProvideScope from inside out
            // Last binding wraps the block, each preceding binding wraps the next
            let mut inner = block_expr;
            for (ctx, als, val) in bindings.into_iter().rev() {
                let inner_span = inner.span;
                let wrap_span = Span::new(start_span.start, inner_span.end, start_span.file_id);
                inner = Expr::new(
                    verum_ast::ExprKind::Block(verum_ast::Block::new(
                        verum_common::List::from(vec![Stmt::new(
                            StmtKind::ProvideScope {
                                context: ctx,
                                alias: als,
                                value: Heap::new(val),
                                block: Heap::new(inner),
                            },
                            wrap_span,
                        )]),
                        Maybe::None,
                        wrap_span,
                    )),
                    wrap_span,
                );
            }

            let span = Span::new(start_span.start, inner.span.end, start_span.file_id);
            return Ok(Stmt::new(
                StmtKind::Expr { expr: inner, has_semi: false },
                span,
            ));
        }

        // Check for block-scoped provide:
        // - 'in' keyword followed by block: `provide Ctx = val in { ... }`
        // - Direct block after value:       `provide Ctx = val { ... }`
        let has_scope_block = if self.stream.check(&TokenKind::In) {
            self.stream.advance(); // consume 'in'
            if self.stream.check(&TokenKind::Semicolon) || self.stream.at_end() {
                return Err(ParseError::provide_invalid(
                    "missing block after 'in' in provide statement",
                    self.stream.current_span(),
                ));
            }
            true
        } else if self.stream.check(&TokenKind::LBrace) {
            // Direct block syntax: `provide Ctx = val { ... }`
            true
        } else {
            false
        };

        if has_scope_block {
            let block = self.parse_block()?;
            let block_span = block.span;
            let block_expr = Expr::new(verum_ast::ExprKind::Block(block), block_span);
            let span = Span::new(start_span.start, block_span.end, start_span.file_id);
            return Ok(Stmt::new(
                StmtKind::ProvideScope {
                    context,
                    alias,
                    value: Heap::new(value),
                    block: Heap::new(block_expr),
                },
                span,
            ));
        }

        // Statement-level provide (no block scope)
        // Semicolon is optional if followed by statement/block terminators
        let end_span = if self.allows_semicolon_omission() {
            if let Some(semi) = self.stream.consume(&TokenKind::Semicolon) {
                semi.span
            } else {
                value_span
            }
        } else {
            let end_token = self.stream.expect(TokenKind::Semicolon)?;
            end_token.span
        };

        let span = Span::new(start_span.start, end_span.end, start_span.file_id);

        Ok(Stmt::new(
            StmtKind::Provide {
                context,
                alias,
                value: Heap::new(value),
            },
            span,
        ))
    }

    /// Parse an expression statement: `expr;` or `expr` (tail position)
    fn parse_expr_stmt(&mut self) -> ParseResult<Stmt> {
        // E048: Check for tokens that DEFINITELY cannot start an expression
        // We explicitly check for invalid tokens rather than using starts_expr()
        // because starts_expr() may not include all valid expression starters
        // (e.g., `nursery` keyword for structured concurrency)
        if let Some(token) = self.stream.peek() {
            let (is_invalid, msg) = match &token.kind {
                // Binary-only operators (cannot be unary)
                TokenKind::Plus => (true, "standalone '+' operator is not a valid expression"),
                TokenKind::Slash => (true, "standalone '/' operator is not a valid expression"),
                TokenKind::Caret => (true, "standalone '^' operator is not a valid expression"),
                // Assignment operators
                TokenKind::Eq => (true, "standalone '=' is not a valid expression; use 'let' for bindings"),
                TokenKind::PlusEq | TokenKind::MinusEq | TokenKind::StarEq | TokenKind::SlashEq => {
                    (true, "assignment operator is not a valid expression start")
                }
                // Comparison operators
                TokenKind::EqEq | TokenKind::BangEq => (true, "standalone comparison operator is not a valid expression"),
                TokenKind::Lt | TokenKind::Gt | TokenKind::LtEq | TokenKind::GtEq => {
                    (true, "standalone comparison operator is not a valid expression")
                }
                // Logical operators (PipePipe is NOT included: || starts an empty closure)
                TokenKind::AmpersandAmpersand => {
                    (true, "standalone logical operator is not a valid expression")
                }
                // Punctuation that can't start expressions
                TokenKind::Comma => (true, "unexpected comma; not a valid expression start"),
                TokenKind::Dot => (true, "unexpected '.'; field access requires a preceding expression"),
                TokenKind::Colon => (true, "unexpected ':'; type annotations require a binding"),
                TokenKind::ColonColon => (true, "unexpected '::'; path requires a preceding identifier"),
                TokenKind::FatArrow => (true, "unexpected '=>'; match arms require 'match'"),
                TokenKind::RArrow => (true, "unexpected '->'; return types require function signature"),
                _ => (false, ""),
            };
            if is_invalid {
                return Err(ParseError::expr_stmt_invalid(msg, token.span));
            }
        }

        let expr = self.parse_expr_adapter()?;
        let span = expr.span;

        // Determine if this is a block-form expression that doesn't require semicolon
        // Block-form expressions end with `}` and naturally terminate
        let is_block_form = self.is_block_form_expr(&expr);

        // Check for semicolon
        let has_semi = self.stream.consume(&TokenKind::Semicolon).is_some();

        // E010: Non-block-form expressions require semicolons unless at tail position
        // Tail position = followed by `}` (end of block) or EOF
        // Error recovery: report error but continue parsing
        if !has_semi && !is_block_form {
            let at_tail_position = self.stream.check(&TokenKind::RBrace) || self.stream.at_end();
            if !at_tail_position {
                // Emit error but continue (error recovery)
                self.error(ParseError::missing_semicolon(
                    self.stream.current_span(),
                    "expression statement",
                ));
            }
        }

        Ok(Stmt::new(StmtKind::Expr { expr, has_semi }, span))
    }

    // ========================================================================
    // Helper Methods
    // ========================================================================

    /// Check if an expression is a "block-form" expression that ends with `}`.
    ///
    /// Block-form expressions naturally terminate and don't require semicolons
    /// even in non-tail position. This includes:
    /// - Block expressions: `{ ... }`
    /// - If expressions: `if cond { ... } else { ... }`
    /// - Match expressions: `match x { ... }`
    /// - Loop expressions: `loop { ... }`, `while cond { ... }`, `for x in iter { ... }`
    /// - Unsafe blocks: `unsafe { ... }`
    /// - Async blocks: `async { ... }`
    /// - Nursery expressions: `nursery { ... }`
    /// - Select expressions: `select { ... }`
    fn is_block_form_expr(&self, expr: &Expr) -> bool {
        use verum_ast::ExprKind;

        match &expr.kind {
            // Basic block expression
            ExprKind::Block(_) => true,
            // Control flow expressions ending with `}`
            ExprKind::If { .. } => true,
            ExprKind::Match { .. } => true,
            ExprKind::Loop { .. } => true,
            ExprKind::While { .. } => true,
            ExprKind::For { .. } => true,
            // Safety/concurrency block expressions
            ExprKind::Unsafe(_) => true,
            ExprKind::Async(_) => true,
            ExprKind::Nursery { .. } => true,
            ExprKind::Select { .. } => true,
            // Try-related expressions (all end with `}`)
            ExprKind::Try(inner) => self.is_block_form_expr(inner),
            ExprKind::TryBlock(_) => true,
            ExprKind::TryRecover { .. } => true,
            ExprKind::TryFinally { .. } => true,
            ExprKind::TryRecoverFinally { .. } => true,
            // Contract literals (contract#"...") are annotation expressions
            // that don't require semicolons when followed by the function body
            ExprKind::Literal(lit) if lit.is_contract() => true,
            _ => false,
        }
    }

    /// Check if the current token is a contextual keyword with the given name.
    fn is_contextual_keyword(&self, name: &str) -> bool {
        if let Some(TokenKind::Ident(ident)) = self.stream.peek_kind() {
            ident.as_str() == name
        } else {
            false
        }
    }

    /// Look ahead to determine if this is a let-else statement.
    ///
    /// We need to distinguish:
    /// Check if the current `@name(...)` construct looks like a macro call expression
    /// rather than an attribute attached to a declaration.
    ///
    /// Rules:
    /// - If followed by `;` → macro call expression statement
    /// - If followed by declaration keyword (type, fn, let, etc.) → attribute
    /// - If the name is a known meta-function → expression (handled by expr parser)
    fn looks_like_macro_call_not_attribute(&self) -> bool {
        // We're at @ token
        let mut stream = self.stream.clone();
        stream.advance(); // consume @

        // Get the identifier/keyword after @
        let name = match stream.peek_kind() {
            Some(TokenKind::Ident(s)) => s.clone(),
            Some(TokenKind::Const) => Text::from("const"),
            Some(TokenKind::Module) => Text::from("module"),
            Some(TokenKind::Fn) => Text::from("function"),
            _ => return false, // Not a valid @ construct
        };
        stream.advance(); // consume name

        // Check if this is a known meta-function - those are handled by expr parser
        // Note: @cfg is NOT in this list because it's primarily used as an attribute
        // for conditional compilation. When @cfg(predicate) { ... } appears at statement
        // position, it should be parsed as an attribute on the block, not as a meta-expression.
        // @cfg can still be used as a boolean expression in contexts like `if @cfg(...) { ... }`
        // because in that case, @cfg is at expression position (handled by expr parser).
        let is_known_meta = matches!(
            name.as_str(),
            "file" | "line" | "column" | "module" | "function"
                | "const" | "error" | "warning" | "stringify" | "concat"
        );
        if is_known_meta {
            return true; // Let expression parser handle it
        }

        // Skip over arguments if present: @name(...)
        if stream.check(&TokenKind::LParen) {
            stream.advance();
            let mut depth = 1;
            while depth > 0 {
                match stream.peek_kind() {
                    None => return false,
                    Some(TokenKind::LParen) => {
                        depth += 1;
                        stream.advance();
                    }
                    Some(TokenKind::RParen) => {
                        depth -= 1;
                        stream.advance();
                    }
                    _ => {
                        stream.advance();
                    }
                }
            }
        }

        // Now check what follows: if it's a statement terminator, it's a macro call
        match stream.peek_kind() {
            Some(TokenKind::Semicolon) => true, // @name(...); → macro call
            Some(TokenKind::RBrace) => true,    // @name(...) } → tail expr
            None => true,                       // End of input
            // If followed by declaration keyword or block, it's an attribute
            Some(TokenKind::Type)
            | Some(TokenKind::Fn)
            | Some(TokenKind::Let)
            | Some(TokenKind::Const)
            | Some(TokenKind::Static)
            | Some(TokenKind::Implement)
            | Some(TokenKind::Mount)
            | Some(TokenKind::For)
            | Some(TokenKind::While)
            | Some(TokenKind::If)
            | Some(TokenKind::Match)
            // LBrace indicates attribute on a block expression (e.g., @cfg(target_os = "linux") { ... })
            | Some(TokenKind::LBrace) => false, // Attribute before statement
            // Another @ - could be stacked attributes
            Some(TokenKind::At) => false,
            // Identifier after @cfg(...) - treat as attribute on the following expression statement
            // e.g., @cfg(debug_assertions) println("debug");
            Some(TokenKind::Ident(_)) => false,
            // pub, async, unsafe, pure etc. before declarations
            Some(TokenKind::Public) | Some(TokenKind::Internal) | Some(TokenKind::Protected)
            | Some(TokenKind::Async) | Some(TokenKind::Unsafe) | Some(TokenKind::Pure)
            | Some(TokenKind::Meta) | Some(TokenKind::Module)
            | Some(TokenKind::Return) | Some(TokenKind::Break) | Some(TokenKind::Continue)
            | Some(TokenKind::Loop) => false,
            // Otherwise, treat as macro call (expression continues)
            _ => true,
        }
    }

    /// Check if the current `@name(...)` looks like a meta-expression statement
    /// rather than an attribute. Used by `parse_attributes()` to stop parsing
    /// attributes when it encounters something like `@asm("lfence");`.
    ///
    /// An `@name(...)` is a meta-expression (not attribute) when:
    /// - Followed by `;` (expression statement)
    /// - Followed by `}` (trailing expression)
    /// - At end of input
    ///
    /// This prevents `@cfg(...) @asm(...);` from parsing @asm as an attribute.
    pub(crate) fn looks_like_meta_expression_not_attribute(&self) -> bool {
        // We're at @ token
        let mut stream = self.stream.clone();
        stream.advance(); // consume @

        // Get the identifier/keyword after @
        match stream.peek_kind() {
            Some(TokenKind::Ident(_))
            | Some(TokenKind::Const)
            | Some(TokenKind::Module)
            | Some(TokenKind::Fn) => {
                stream.advance();
            }
            _ => return false, // Not a valid @ construct
        }

        // Skip over arguments if present: @name(...)
        if stream.check(&TokenKind::LParen) {
            stream.advance();
            let mut depth = 1;
            while depth > 0 {
                match stream.peek_kind() {
                    None => return false,
                    Some(TokenKind::LParen) => {
                        depth += 1;
                        stream.advance();
                    }
                    Some(TokenKind::RParen) => {
                        depth -= 1;
                        stream.advance();
                    }
                    _ => {
                        stream.advance();
                    }
                }
            }
        }

        // Check what follows: if it's an expression terminator, it's NOT an attribute
        matches!(
            stream.peek_kind(),
            Some(TokenKind::Semicolon) | Some(TokenKind::RBrace) | None
        )
    }

    /// - `let x = expr else { ... }` (let-else)
    /// - `let x = expr;` (regular let)
    ///
    /// This requires scanning ahead to find the `else` keyword after the initializer.
    fn looks_like_let_else(&self) -> bool {
        let saved_pos = self.stream.position();
        let mut stream = self.stream.clone();

        // Skip past 'let'
        stream.advance();

        // Scan past the pattern by tracking delimiter depth until we find '=' at depth 0.
        // This approach (delimiter scanning) is more robust than trying to fully parse
        // the pattern, which would be complex and error-prone for lookahead purposes.
        let mut depth = 0;
        loop {
            match stream.peek_kind() {
                None | Some(TokenKind::Semicolon) => {
                    return false;
                }
                Some(TokenKind::LParen) | Some(TokenKind::LBracket) | Some(TokenKind::LBrace) => {
                    depth += 1;
                    stream.advance();
                }
                Some(TokenKind::RParen) | Some(TokenKind::RBracket) | Some(TokenKind::RBrace) => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                    stream.advance();
                }
                Some(TokenKind::Eq) => {
                    if depth == 0 {
                        break;
                    }
                    stream.advance();
                }
                Some(TokenKind::Colon) => {
                    if depth == 0 {
                        // Type annotation, keep scanning
                        stream.advance();
                        continue;
                    }
                    stream.advance();
                }
                _ => {
                    stream.advance();
                }
            }
        }

        // Now scan for 'else' keyword after expression
        stream.advance(); // Skip '='

        // Skip the expression (scan for 'else' at depth 0)
        // We need to stop at statement-starting keywords when depth == 0
        // because those indicate a new statement, not continuation of let-else
        depth = 0;
        loop {
            match stream.peek_kind() {
                None | Some(TokenKind::Semicolon) => {
                    return false;
                }
                // Stop at statement-starting keywords at depth 0
                // These indicate a new statement, not let-else continuation
                Some(TokenKind::If)
                | Some(TokenKind::While)
                | Some(TokenKind::For)
                | Some(TokenKind::Match)
                | Some(TokenKind::Return)
                | Some(TokenKind::Let)
                | Some(TokenKind::Loop)
                | Some(TokenKind::Break)
                | Some(TokenKind::Continue)
                | Some(TokenKind::Defer)
                | Some(TokenKind::Errdefer)
                | Some(TokenKind::Fn)
                | Some(TokenKind::Type)
                    if depth == 0 =>
                {
                    return false;
                }
                Some(TokenKind::Else) if depth == 0 => {
                    return true;
                }
                Some(TokenKind::LParen) | Some(TokenKind::LBracket) | Some(TokenKind::LBrace) => {
                    depth += 1;
                    stream.advance();
                }
                Some(TokenKind::RParen) | Some(TokenKind::RBracket) | Some(TokenKind::RBrace) => {
                    if depth == 0 {
                        // Hit closing delimiter without matching opening - not let-else
                        // This handles the case: `if x { let y = 1 } else { ... }`
                        // where the } is from the if block, not part of let-else
                        return false;
                    }
                    depth -= 1;
                    stream.advance();
                }
                _ => {
                    stream.advance();
                }
            }

            // Safety: don't scan too far
            if stream.position() > saved_pos + 100 {
                return false;
            }
        }
    }

    // ========================================================================
    // Adapter Methods for Sub-Parsers
    // ========================================================================
    //
    // These methods provide a clean interface for parsing expressions, types,
    // and patterns within statement contexts.

    /// Parse an expression using the hand-written parser.
    fn parse_expr_adapter(&mut self) -> ParseResult<Expr> {
        // expr.rs has been migrated to hand-written parsing
        // Just delegate to parse_expr() which is defined in expr.rs
        self.parse_expr()
    }

    /// Parse a pattern using the hand-written parser.
    fn parse_pattern_adapter(&mut self) -> ParseResult<Pattern> {
        // Use the hand-written parser defined in pattern.rs
        self.parse_pattern()
    }

    /// Parse a type using the hand-written parser.
    fn parse_type_adapter(&mut self) -> ParseResult<Type> {
        // Use the hand-written parser defined in ty.rs
        self.parse_type()
    }

    /// Advance stream past an expression.
    ///
    /// This is a heuristic approach - we scan forward until we hit a likely
    /// expression boundary (semicolon, closing delimiter, keyword, etc.)
    fn advance_past_expr(&mut self) {
        let mut depth = 0;

        loop {
            match self.stream.peek_kind() {
                None => break,
                Some(TokenKind::Semicolon) if depth == 0 => break,
                Some(TokenKind::RBrace) if depth == 0 => break,
                Some(TokenKind::Else) if depth == 0 => break,
                Some(TokenKind::LParen) | Some(TokenKind::LBracket) | Some(TokenKind::LBrace) => {
                    depth += 1;
                    self.stream.advance();
                }
                Some(TokenKind::RParen) | Some(TokenKind::RBracket) => {
                    if depth > 0 {
                        depth -= 1;
                        self.stream.advance();
                    } else {
                        break;
                    }
                }
                Some(TokenKind::Comma) if depth == 0 => break,
                _ => {
                    self.stream.advance();
                }
            }
        }
    }

    /// Advance stream past a pattern.
    fn advance_past_pattern(&mut self) {
        let mut depth = 0;

        loop {
            match self.stream.peek_kind() {
                None => break,
                Some(TokenKind::Eq) if depth == 0 => break,
                Some(TokenKind::Colon) if depth == 0 => break,
                Some(TokenKind::In) if depth == 0 => break,
                Some(TokenKind::LParen) | Some(TokenKind::LBracket) | Some(TokenKind::LBrace) => {
                    depth += 1;
                    self.stream.advance();
                }
                Some(TokenKind::RParen) | Some(TokenKind::RBracket) | Some(TokenKind::RBrace) => {
                    if depth > 0 {
                        depth -= 1;
                        self.stream.advance();
                    } else {
                        break;
                    }
                }
                _ => {
                    self.stream.advance();
                }
            }
        }
    }

    /// Advance stream past a type.
    fn advance_past_type(&mut self) {
        let mut depth = 0;

        loop {
            match self.stream.peek_kind() {
                None => break,
                Some(TokenKind::Eq) if depth == 0 => break,
                Some(TokenKind::Semicolon) if depth == 0 => break,
                Some(TokenKind::Comma) if depth == 0 => break,
                Some(TokenKind::RBrace) if depth == 0 => break,
                Some(TokenKind::LParen)
                | Some(TokenKind::LBracket)
                | Some(TokenKind::LBrace)
                | Some(TokenKind::Lt) => {
                    depth += 1;
                    self.stream.advance();
                }
                Some(TokenKind::RParen)
                | Some(TokenKind::RBracket)
                | Some(TokenKind::RBrace)
                | Some(TokenKind::Gt) => {
                    if depth > 0 {
                        depth -= 1;
                        self.stream.advance();
                    } else {
                        break;
                    }
                }
                _ => {
                    self.stream.advance();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::FileId;
    use verum_lexer::{Lexer, Token};

    fn parse_stmt(source: &str) -> Result<Stmt, ParseError> {
        let file_id = FileId::new(0);
        let lexer = Lexer::new(source, file_id);
        let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = RecursiveParser::new(&tokens, file_id);
        parser.parse_stmt()
    }

    #[test]
    fn test_empty_stmt_is_error() {
        // VCS specifies that empty statements (standalone semicolons) are E048 errors
        let result = parse_stmt(";");
        assert!(result.is_err());
    }

    #[test]
    fn test_let_stmt_simple() {
        let result = parse_stmt("let x = 42;");
        assert!(result.is_ok());
        if let Ok(stmt) = result {
            assert!(matches!(stmt.kind, StmtKind::Let { .. }));
        }
    }

    #[test]
    fn test_let_stmt_with_type() {
        let result = parse_stmt("let x: Int = 42;");
        assert!(result.is_ok());
    }

    #[test]
    fn test_defer_stmt() {
        let result = parse_stmt("defer close(file);");
        assert!(result.is_ok());
        if let Ok(stmt) = result {
            assert!(matches!(stmt.kind, StmtKind::Defer(_)));
        }
    }

    #[test]
    fn test_errdefer_stmt_expr() {
        // Test errdefer with expression form
        let result = parse_stmt("errdefer file.close();");
        assert!(result.is_ok());
        if let Ok(stmt) = result {
            assert!(matches!(stmt.kind, StmtKind::Errdefer(_)));
        }
    }

    #[test]
    fn test_errdefer_stmt_block() {
        // Test errdefer with block form
        let result = parse_stmt("errdefer { cleanup(); log_error(); }");
        assert!(result.is_ok());
        if let Ok(stmt) = result {
            assert!(matches!(stmt.kind, StmtKind::Errdefer(_)));
        }
    }

    #[test]
    fn test_errdefer_stmt_simple_call() {
        // Test errdefer with a simple function call
        let result = parse_stmt("errdefer cleanup();");
        assert!(result.is_ok());
        if let Ok(stmt) = result {
            assert!(matches!(stmt.kind, StmtKind::Errdefer(_)));
        }
    }

    #[test]
    fn test_provide_stmt() {
        let result = parse_stmt("provide Database = db;");
        assert!(result.is_ok());
        if let Ok(stmt) = result {
            assert!(matches!(stmt.kind, StmtKind::Provide { .. }));
            if let StmtKind::Provide { context, .. } = stmt.kind {
                assert_eq!(context.as_str(), "Database");
            }
        }
    }

    #[test]
    fn test_provide_stmt_path() {
        let result = parse_stmt("provide FileSystem.Write = writer;");
        assert!(result.is_ok());
        if let Ok(stmt) = result {
            assert!(matches!(stmt.kind, StmtKind::Provide { .. }));
            if let StmtKind::Provide { context, .. } = stmt.kind {
                assert_eq!(context.as_str(), "FileSystem.Write");
            }
        }
    }

    #[test]
    fn test_provide_scope_stmt() {
        let result = parse_stmt("provide Database = db in { query_users() }");
        assert!(result.is_ok());
        if let Ok(stmt) = result {
            assert!(matches!(stmt.kind, StmtKind::ProvideScope { .. }));
            if let StmtKind::ProvideScope { context, .. } = stmt.kind {
                assert_eq!(context.as_str(), "Database");
            }
        }
    }

    #[test]
    fn test_provide_scope_stmt_complex() {
        let result = parse_stmt("provide Logger = my_logger in { log(\"test\"); process() }");
        assert!(result.is_ok());
        if let Ok(stmt) = result {
            assert!(matches!(stmt.kind, StmtKind::ProvideScope { .. }));
        }
    }

    // ========================================================================
    // Provider Alias Tests (context provide with 'as' alias for multiple instances)
    // ========================================================================

    #[test]
    fn test_provide_stmt_with_alias() {
        // Test provide with alias: provide Database as source = db;
        let result = parse_stmt("provide Database as source = db;");
        assert!(result.is_ok());
        if let Ok(stmt) = result {
            if let StmtKind::Provide { context, alias, .. } = stmt.kind {
                assert_eq!(context.as_str(), "Database");
                assert!(alias.is_some());
                assert_eq!(alias.unwrap().as_str(), "source");
            } else {
                panic!("Expected Provide statement");
            }
        }
    }

    #[test]
    fn test_provide_stmt_with_alias_complex_value() {
        // Test provide with alias and complex value expression
        // Note: Verum uses `.` for path access, not `::`
        let result = parse_stmt("provide Database as target = PostgresHandler.connect(url);");
        assert!(result.is_ok());
        if let Ok(stmt) = result {
            if let StmtKind::Provide { context, alias, .. } = stmt.kind {
                assert_eq!(context.as_str(), "Database");
                assert!(alias.is_some());
                assert_eq!(alias.unwrap().as_str(), "target");
            } else {
                panic!("Expected Provide statement");
            }
        }
    }

    #[test]
    fn test_provide_stmt_without_alias() {
        // Ensure non-aliased provide still works
        let result = parse_stmt("provide Logger = console_logger;");
        assert!(result.is_ok());
        if let Ok(stmt) = result {
            if let StmtKind::Provide { context, alias, .. } = stmt.kind {
                assert_eq!(context.as_str(), "Logger");
                assert!(alias.is_none());
            } else {
                panic!("Expected Provide statement");
            }
        }
    }

    #[test]
    fn test_provide_scope_with_alias() {
        // Test block-scoped provide with alias
        let result = parse_stmt("provide Database as backup = db in { backup_data() }");
        assert!(result.is_ok());
        if let Ok(stmt) = result {
            if let StmtKind::ProvideScope { context, alias, .. } = stmt.kind {
                assert_eq!(context.as_str(), "Database");
                assert!(alias.is_some());
                assert_eq!(alias.unwrap().as_str(), "backup");
            } else {
                panic!("Expected ProvideScope statement");
            }
        }
    }

    #[test]
    fn test_provide_path_with_alias() {
        // Test path-based context with alias
        let result = parse_stmt("provide FileSystem.Write as logs = log_writer;");
        assert!(result.is_ok());
        if let Ok(stmt) = result {
            if let StmtKind::Provide { context, alias, .. } = stmt.kind {
                assert_eq!(context.as_str(), "FileSystem.Write");
                assert!(alias.is_some());
                assert_eq!(alias.unwrap().as_str(), "logs");
            } else {
                panic!("Expected Provide statement");
            }
        }
    }

    // ========================================================================
    // Let-else Tests
    // ========================================================================

    #[test]
    fn test_let_else_basic() {
        // Basic let-else statement
        let result = parse_stmt("let Some(value) = maybe else { return; };");
        assert!(result.is_ok(), "Failed to parse let-else: {:?}", result.err());
        if let Ok(stmt) = result {
            assert!(matches!(stmt.kind, StmtKind::LetElse { .. }), "Expected LetElse, got {:?}", stmt.kind);
        }
    }

    #[test]
    fn test_let_else_ok_pattern() {
        let result = parse_stmt("let Ok(data) = result else { return default(); };");
        assert!(result.is_ok(), "Failed to parse let-else: {:?}", result.err());
        if let Ok(stmt) = result {
            assert!(matches!(stmt.kind, StmtKind::LetElse { .. }));
        }
    }

    #[test]
    fn test_let_else_complex_pattern() {
        let result = parse_stmt(r#"let Some(Point { x, y }) = maybe_point else { panic("Expected point"); };"#);
        assert!(result.is_ok(), "Failed to parse let-else: {:?}", result.err());
        if let Ok(stmt) = result {
            assert!(matches!(stmt.kind, StmtKind::LetElse { .. }));
        }
    }
}
