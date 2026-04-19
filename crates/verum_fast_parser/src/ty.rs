//! Type parser for Verum using hand-written recursive descent.
//!
//! This module implements parsing for all Verum types, including:
//! - Primitive types (Int, Float, Bool, Text, Char)
//! - Compound types (tuples, arrays, functions)
//! - Refinement types: `Int{> 0}`, `Text{is_email(it)}`
//! - Generic types: `List<T>`, `HashMap<K, V>`
//! - References: `&T` (CBGR), `&checked T`, `&unsafe T`
//! - Function types: `fn(A, B) -> C`

use verum_ast::ffi::CallingConvention;
use verum_ast::ty::{GenericParamKind, KindAnnotation, Lifetime, WherePredicate, WherePredicateKind};
use verum_ast::{
    ContextList, ContextRequirement, GenericParam, Ident, Path, RefinementPredicate, Type,
    TypeKind, WhereClause, ty::*,
};
use verum_common::{Heap, List, Maybe, Text};
use verum_lexer::{Token, TokenKind};

use crate::error::{ErrorCode, ParseError, ParseErrorKind};
use crate::parser::{ParseResult, RecursiveParser};

impl<'a> RecursiveParser<'a> {
    /// Parse a type expression.
    ///
    /// This is the main entry point for type parsing.
    pub fn parse_type(&mut self) -> ParseResult<Type> {
        self.parse_type_impl(true, false, true)
    }

    /// Parse a type without refinement predicates.
    /// Used in contexts where { } is not a refinement (e.g., implement blocks).
    pub fn parse_type_no_refinement(&mut self) -> ParseResult<Type> {
        self.parse_type_impl(false, false, true)
    }

    /// Parse a type that may have a refinement, but check lookahead to avoid
    /// consuming a function/impl body brace.
    ///
    /// This is used for return types in functions, predicates, and impl methods where:
    /// - `fn abs(x: Int) -> Int{>= 0} { body }` should parse the refinement
    /// - `fn is_even(n: Int) -> Bool { body }` should NOT consume the body brace as refinement
    ///
    /// The key: refinements can only appear if there's content inside the braces that looks
    /// like a predicate (starts with comparison operator, or is an expression).
    pub fn parse_type_with_lookahead(&mut self) -> ParseResult<Type> {
        self.parse_type_impl(true, true, true)
    }

    /// Parse a type without sigma type interpretation.
    /// Used in where clause contexts where `T: Bound` is a type bound, not a sigma type.
    /// This prevents `where type T: Clone` from being parsed as a sigma type.
    pub fn parse_type_no_sigma(&mut self) -> ParseResult<Type> {
        self.parse_type_impl(true, false, false)
    }

    /// Parse a simple type for quantifier bindings.
    /// This is a non-path-extending type that doesn't consume `.` for associated types.
    /// Used in `forall x: T . body` and `exists x: T . body` where `.` separates the
    /// binding from the body expression, not part of the type.
    pub fn parse_type_for_quantifier(&mut self) -> ParseResult<Type> {
        // Save the current position to potentially backtrack
        let start_pos = self.stream.position();

        // For quantifier bindings, we only want simple types (no path continuation with `.`)
        // Parse primitives, path types without `.`, function types, etc.
        // but don't allow T.x to be parsed as associated type

        // Try primitives first
        if let Some(prim) = self.optional(|p| p.parse_primitive_type()) {
            // Check for inline refinement: Int{> 0}, Float{!= 0.0}, etc.
            if self.stream.check(&TokenKind::LBrace) {
                let predicate = self.parse_refinement_predicate()?;
                let span = self.stream.make_span(start_pos);
                return Ok(Type::new(
                    TypeKind::Refined {
                        base: Box::new(prim),
                        predicate: Box::new(predicate),
                    },
                    span,
                ));
            }
            return Ok(prim);
        }

        // Unit type: ()
        if self.stream.check(&TokenKind::LParen)
            && let Some(unit) = self.optional(|p| {
                p.stream.expect(TokenKind::LParen)?;
                p.stream.expect(TokenKind::RParen)?;
                let span = p.stream.make_span(start_pos);
                Ok(Type::unit(span))
            })
        {
            return Ok(unit);
        }

        // Never type: !
        if self.stream.check(&TokenKind::Bang) {
            self.stream.advance();
            let span = self.stream.make_span(start_pos);
            return Ok(Type::never(span));
        }

        // Tuple type: (A, B) - parse without consuming subsequent `.`
        if self.stream.check(&TokenKind::LParen)
            && let Some(tuple) = self.optional(|p| p.parse_tuple_type())
        {
            return Ok(tuple);
        }

        // Array/slice type: [T; N] or [T]
        if self.stream.check(&TokenKind::LBracket) {
            return self.parse_array_or_slice_type();
        }

        // Reference types without path extension
        if self.stream.check_any(&[
            TokenKind::Ampersand,
            TokenKind::Percent,
            TokenKind::Star,
            TokenKind::StarStar,
            TokenKind::AmpersandAmpersand,
        ]) {
            return self.parse_reference_type();
        }

        // For simple identifiers/path types: parse just the base identifier without `.` continuation
        // This is the key difference from parse_path_type - we don't allow T.Item syntax
        if self.is_ident()
            || self.stream.check(&TokenKind::SelfValue)
            || self.stream.check(&TokenKind::SelfType)
        {
            return self.parse_simple_path_type_for_quantifier();
        }

        // Function type
        if self.stream.check(&TokenKind::Fn) {
            return self.parse_function_type(Maybe::None);
        }

        let span = self.stream.current_span();
        Err(ParseError::invalid_syntax("expected type in quantifier binding", span))
    }

    /// Parse a simple path-based type without `.` continuation.
    /// Used for quantifier bindings where `.` separates binding from body.
    fn parse_simple_path_type_for_quantifier(&mut self) -> ParseResult<Type> {
        let start_pos = self.stream.position();

        // Parse the type name
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        let span = self.stream.make_span(start_pos);

        // Create a simple path type (single segment, no `.` continuation)
        let path = Path::new(
            vec![PathSegment::Name(Ident::new(name, name_span))]
                .into_iter()
                .collect(),
            span,
        );

        let base_type = Type::new(TypeKind::Path(path), span);

        // Check for generic arguments: T<U>
        let base_type = if self.stream.check(&TokenKind::Lt) {
            let args = self.parse_generic_args()?;
            let span = self.stream.make_span(start_pos);

            Type::new(
                TypeKind::Generic {
                    base: Box::new(base_type),
                    args: args.into_iter().collect::<List<_>>(),
                },
                span,
            )
        } else {
            base_type
        };

        // Check for inline refinement: T{predicate} (e.g., Int{> 0 && < m})
        if self.stream.check(&TokenKind::LBrace) {
            let predicate = self.parse_refinement_predicate()?;
            let span = self.stream.make_span(start_pos);
            Ok(Type::new(
                TypeKind::Refined {
                    base: Box::new(base_type),
                    predicate: Box::new(predicate),
                },
                span,
            ))
        } else {
            Ok(base_type)
        }
    }

    fn parse_type_impl(
        &mut self,
        allow_refinement: bool,
        use_lookahead: bool,
        allow_sigma: bool,
    ) -> ParseResult<Type> {
        self.enter_recursion()?;
        let result = self.parse_type_impl_inner(allow_refinement, use_lookahead, allow_sigma);
        self.exit_recursion();
        result
    }

    fn parse_type_impl_inner(
        &mut self,
        allow_refinement: bool,
        use_lookahead: bool,
        allow_sigma: bool,
    ) -> ParseResult<Type> {
        let start_pos = self.stream.position();

        // Try to parse sigma-type first: name: Type where expr
        // This must be parsed before general types because it's more specific
        // Use lookahead to avoid infinite recursion: check for "Ident : " pattern
        // We use optional() to allow backtracking if it's not actually a sigma type
        // Skip sigma parsing in where clause contexts where `T: Bound` is a type bound
        if allow_sigma
            && self.looks_like_sigma_type()
            && let Some(sigma) = self.optional(|p| p.parse_sigma_type())
        {
            return Ok(sigma);
        }

        // Parse base type
        let mut base = self.parse_base_type()?;

        // Check for capability-restricted type: Type with [Capabilities]
        // Capability-restricted types: `Type with [Cap1, Cap2]` restricts a context
        // to a subset of its capabilities (e.g., `Database with [Read]` allows only reads)
        //
        // This must be checked before refinements because:
        // - `Database with [Read]` is a capability type
        // - `Database{is_connected}` is a refinement type
        // - `Database with [Read]{is_connected}` is a capability type with refinement
        if self.stream.check(&TokenKind::With) {
            self.stream.advance(); // consume 'with'
            let capabilities = self.parse_capability_list()?;
            let span = self.stream.make_span(start_pos);
            base = Type::new(
                TypeKind::CapabilityRestricted {
                    base: Box::new(base),
                    capabilities,
                },
                span,
            );
        }
        // E077: Capability syntax without 'with' keyword: `Database [Read]` instead of `Database with [Read]`
        // Detect when a type is followed directly by `[` which looks like a capability list
        // NOTE: Skip this check when use_lookahead is true (return type context), because
        // `[...]` after a return type can be context requirements: `fn foo() -> Text [IO] { }`
        else if !use_lookahead && self.stream.check(&TokenKind::LBracket) {
            // Look ahead to see if this looks like a capability list (identifier followed by , or ])
            // This helps distinguish from array indexing or other syntax
            let pos = self.stream.position();
            self.stream.advance(); // consume [
            let looks_like_capability = matches!(
                self.stream.peek_kind(),
                Some(TokenKind::Ident(_)) | Some(TokenKind::RBracket)
            );
            self.stream.reset_to(pos); // restore position
            if looks_like_capability {
                return Err(ParseError::capability_no_with(self.stream.make_span(start_pos)));
            }
        }

        // Apply refinement predicate if present: Type{...} or Type where ...
        // Only if refinements are allowed in this context
        // Note: We need to distinguish `Type where predicate` (refinement) from declaration-level where clauses
        //
        // IMPORTANT: When use_lookahead is true (for function/impl return types), we DON'T allow
        // `where` refinements because `where` after a return type is almost always a generic where clause:
        // - `fn foo() -> Vec<U> where F: Fn(T) -> U { ... }` - generic where clause, not refinement
        // - `type Pos is Int where x > 0;` - refinement where clause (but not in return type context)
        //
        // EXCEPTION: Lambda refinements like `where |x| x > 0` are ALWAYS refinements, never generic
        // where clauses, because generic where clauses don't have lambda syntax. So we allow them.
        //
        // So `where` refinements are only allowed when:
        // 1. use_lookahead is false (type alias context), OR
        // 2. The where is followed by a lambda: where |x| expr
        let has_where_refinement = self.stream.check(&TokenKind::Where) && {
            // Check if it's a refinement `where` or a declaration-level where clause
            // If next token is `meta` or `type`, it's a declaration-level where clause
            // Note: `where value` and `where ensures` are handled in refinement parsing
            let is_declaration_where = matches!(
                self.stream.peek_nth(1).map(|t| &t.kind),
                Some(TokenKind::Meta) | Some(TokenKind::Type) | Some(TokenKind::Ensures)
            );

            // If it's a declaration-level where clause, don't treat as refinement
            if is_declaration_where {
                false
            } else {
                // Check if it's a lambda refinement: where |x| expr
                let is_lambda_refinement = matches!(
                    self.stream.peek_nth(1).map(|t| &t.kind),
                    Some(TokenKind::Pipe)
                );

                // Allow if: not using lookahead, OR it's a lambda refinement
                !use_lookahead || is_lambda_refinement
            }
        };

        if allow_refinement && (self.stream.check(&TokenKind::LBrace) || has_where_refinement) {
            // For `where` refinements, we can parse directly without lookahead
            // because `where` is unambiguous (we already checked it's not `where meta` or `where type`)
            if has_where_refinement {
                let predicate = self.parse_refinement_predicate()?;
                let span = self.stream.make_span(start_pos);
                return Ok(Type::new(
                    TypeKind::Refined {
                        base: Box::new(base),
                        predicate: Box::new(predicate),
                    },
                    span,
                ));
            }

            // For `{` braces, we need lookahead if requested
            // If use_lookahead is true, we need to be more careful.
            // We want to parse refinements like `Int{>= 0}` but not consume
            // function/impl body braces.
            //
            // Strategy: Look ahead into the braces to see if it looks like a refinement.
            // Refinements start with:
            // - Comparison operators: >, <, >=, <=, ==, !=
            // - Identifiers (function calls or variable names)
            // - Pipe (lambda syntax: where |x| ...)
            //
            // Function bodies start with:
            // - Statements (let, if, return, loop, etc.)
            // - Block expressions
            // - RBrace (empty body)

            if use_lookahead {
                let checkpoint = self.stream.position();

                // Peek ahead: is the next token after { indicative of a refinement?
                self.stream.advance(); // consume {

                let looks_like_refinement = match self.stream.peek_kind() {
                    // Empty braces: could be empty body {}, definitely not a refinement
                    Some(TokenKind::RBrace) => false,
                    // Comparison operators: definitely a refinement {>= 0}, {< 100}
                    Some(TokenKind::Gt)
                    | Some(TokenKind::Lt)
                    | Some(TokenKind::GtEq)
                    | Some(TokenKind::LtEq)
                    | Some(TokenKind::EqEq)
                    | Some(TokenKind::BangEq) => true,
                    // Negation operator: could be refinement {!it.starts_with('/')}
                    // or function body { !foo() }
                    // Only treat as refinement if the token after ! is 'it'
                    Some(TokenKind::Bang) => {
                        // Look at what follows the !
                        match self.stream.peek_nth(1).map(|t| &t.kind) {
                            Some(TokenKind::Ident(name)) if name.as_str() == "it" => true,
                            _ => false, // Conservatively assume function body for { !foo() }, { !true }, etc.
                        }
                    }
                    // Statement keywords: definitely a function body
                    Some(TokenKind::Let)
                    | Some(TokenKind::If)
                    | Some(TokenKind::Return)
                    | Some(TokenKind::Loop)
                    | Some(TokenKind::While)
                    | Some(TokenKind::For)
                    | Some(TokenKind::Match)
                    | Some(TokenKind::Break)
                    | Some(TokenKind::Continue) => false,
                    // Identifiers are tricky. In practice:
                    // - Refinements: {is_valid(it)}, {it > 0}
                    // - Function bodies: {x + y}, {n % 2 == 0}, { x }
                    //
                    // Conservative approach: assume identifier = function body
                    // Only treat as refinement if followed immediately by `(`
                    // (indicating a predicate call like {is_valid(it)})
                    //
                    // Note: {foo} could be either a refinement or a function body.
                    // In function return type context, it's more likely a function body,
                    // so we treat it conservatively as a function body.
                    // Handle 'result' keyword for return type refinements like Int{result > 0}
                    // Note: 'result' is lexed as TokenKind::Result (a keyword), not as an identifier
                    Some(TokenKind::Result) => {
                        match self.stream.peek_nth(1).map(|t| &t.kind) {
                            // Comparison operators suggest refinement predicates
                            Some(TokenKind::Gt)
                            | Some(TokenKind::Lt)
                            | Some(TokenKind::GtEq)
                            | Some(TokenKind::LtEq)
                            | Some(TokenKind::EqEq)
                            | Some(TokenKind::BangEq) => true,
                            // Logical operators
                            Some(TokenKind::AmpersandAmpersand) | Some(TokenKind::PipePipe) => {
                                true
                            }
                            // Pattern matching: {result is Some => ...}
                            Some(TokenKind::Is) => true,
                            // result.xxx is ambiguous: could be refinement {result.len() > 0}
                            // or function body {result.map(|x| x + 1)}. In return type context,
                            // treat as function body since method call chains are more common.
                            // Users should write `result > 0` for simple refinements.
                            Some(TokenKind::Dot) => false,
                            _ => false,
                        }
                    }
                    // Handle 'self' for tuple/struct refinements like (Int, Int) { self.0 <= self.1 }
                    //
                    // IMPORTANT: In return type context (use_lookahead=true), `{ self ... }` is
                    // almost always a function body, not a refinement:
                    // - `fn method(&self) -> Bool { self == other }` - function body
                    // - `fn method(&self) -> Bool { self.x >= 0 }` - function body
                    //
                    // Refinements on return types use `it` or `result`, not `self`:
                    // - `fn foo() -> Int { it > 0 }` - refinement
                    // - `fn foo() -> Int { result >= 0 }` - refinement
                    //
                    // Type alias contexts (use_lookahead=false) can have `self` refinements:
                    // - `type Pos is (Int, Int) { self.0 >= 0 && self.1 >= 0 };`
                    //
                    // Since we're in use_lookahead=true context here, always treat { self ... }
                    // as a function body to avoid the confusing parse error where the function
                    // body is consumed as a refinement type.
                    Some(TokenKind::SelfValue) => false,
                    Some(TokenKind::Ident(name)) => {
                        // VERY conservative heuristic: Only treat as refinement if it's explicitly using 'it'
                        // This avoids false positives like `Bool { x > 0 }` (function body) vs `Int{it > 0}` (refinement)
                        //
                        // Refinements typically use the implicit `it` variable: {it > 0}, {it.contains('@')}
                        // For 'result', it's handled as TokenKind::Result above (lexed as a keyword)
                        // Function bodies use parameter names: {x > 0}, {n % 2 == 0}
                        //
                        // If the identifier is 'it', check the next token:
                        if name.as_str() == "it" {
                            match self.stream.peek_nth(1).map(|t| &t.kind) {
                                Some(TokenKind::Dot) => true, // Method call: {it.contains}
                                // Comparison operators suggest refinement predicates
                                Some(TokenKind::Gt)
                                | Some(TokenKind::Lt)
                                | Some(TokenKind::GtEq)
                                | Some(TokenKind::LtEq)
                                | Some(TokenKind::EqEq)
                                | Some(TokenKind::BangEq) => true,
                                // Logical operators
                                Some(TokenKind::AmpersandAmpersand) | Some(TokenKind::PipePipe) => {
                                    true
                                }
                                // Pattern matching: {it is Some => ...}
                                Some(TokenKind::Is) => true,
                                _ => false,
                            }
                        } else {
                            // Not 'it' - conservatively assume function body
                            false
                        }
                    }
                    // Other tokens: conservatively assume function body
                    _ => false,
                };

                // Reset to before the {
                self.stream.reset_to(checkpoint);

                if looks_like_refinement {
                    // Parse as refinement
                    let predicate = self.parse_refinement_predicate()?;
                    let span = self.stream.make_span(start_pos);
                    return Ok(Type::new(
                        TypeKind::Refined {
                            base: Box::new(base),
                            predicate: Box::new(predicate),
                        },
                        span,
                    ));
                }
                // Otherwise, don't consume the { - it's a function body
            } else {
                // No lookahead: just parse the refinement normally
                let predicate = self.parse_refinement_predicate()?;
                let span = self.stream.make_span(start_pos);
                return Ok(Type::new(
                    TypeKind::Refined {
                        base: Box::new(base),
                        predicate: Box::new(predicate),
                    },
                    span,
                ));
            }
        }

        Ok(base)
    }

    /// Check if the current position looks like a sigma-type pattern: `name: Type where expr`
    /// Uses lookahead to avoid consuming tokens. Returns true only if we see `Ident :` or `result :` pattern.
    fn looks_like_sigma_type(&self) -> bool {
        // Check for: (Ident | result) Colon ... pattern
        // This distinguishes sigma types from regular path types
        // Note: `result` is a keyword used in sigma types like `-> result: Int where result > 0`
        let is_name = matches!(
            self.stream.peek().map(|t| &t.kind),
            Some(TokenKind::Ident(_)) | Some(TokenKind::Result)
        );
        let is_colon = matches!(
            self.stream.peek_nth(1).map(|t| &t.kind),
            Some(TokenKind::Colon)
        );

        is_name && is_colon
    }

    /// Parse a sigma-type: name: Type where expr
    /// Sigma-type (dependent pair): `name: Type where predicate`
    /// Rule 3 of the Five Binding Rules: sigma-type form is canonical for dependent types.
    /// Three equivalent refinement forms: inline `T{pred}`, declarative `T where pred`,
    /// and sigma `n: T where f(n)`. The sigma form binds a name visible in the predicate.
    pub fn parse_sigma_type(&mut self) -> ParseResult<Type> {
        let start_pos = self.stream.position();

        // Parse: name : Type where expr
        // Note: name can be an identifier or the `result` keyword
        let name = if self.stream.check(&TokenKind::Result) {
            self.stream.advance();
            Text::from("result")
        } else {
            self.consume_ident()?
        };
        let name_span = self.stream.current_span();

        self.stream.expect(TokenKind::Colon)?;
        // Parse base type without allowing refinements (to avoid consuming the 'where' clause)
        let base = self.parse_type_no_refinement()?;

        // E081: Detect and reject chained colon syntax like `Item: Display: Clone`
        // This happens when the base type is itself a sigma type without wrapping parens.
        // Valid: `inner: (n: Int, data: [Int])` - nested sigma wrapped in parens
        // Invalid: `Item: Display: Clone` - chained colons without parens
        if matches!(base.kind, TypeKind::Sigma { .. }) {
            return Err(ParseError::unclosed_constraint_generic(base.span));
        }

        // Optional where clause per grammar: sigma_binding = identifier , ':' , type_expr , [ 'where' , expression ] ;
        let predicate = if self.stream.consume(&TokenKind::Where).is_some() {
            // Parse predicate expression
            self.parse_expr()?
        } else {
            // No where clause - use `true` as the predicate
            let span = self.stream.current_span();
            verum_ast::Expr::new(
                verum_ast::ExprKind::Literal(verum_ast::Literal::bool(true, span)),
                span,
            )
        };

        let span = self.stream.make_span(start_pos);
        Ok(Type::new(
            TypeKind::Sigma {
                name: Ident::new(name, name_span),
                base: Box::new(base),
                predicate: Box::new(predicate),
            },
            span,
        ))
    }

    /// Parse a base type (without refinements).
    fn parse_base_type(&mut self) -> ParseResult<Type> {
        let start_pos = self.stream.position();

        // Type lambda: |x| TypeExpr or |x, y| TypeExpr
        // Used in dependent type positions: Sigma<A, |a| B(a)>
        // Try to parse as type lambda first; if it fails, report the original error.
        if self.stream.check(&TokenKind::Pipe) {
            if let Some(type_lambda) = self.optional(|p| p.parse_type_lambda()) {
                return Ok(type_lambda);
            }
            // If type lambda parsing failed, report the original error
            return Err(ParseError::leading_pipe_no_context(self.stream.current_span()));
        }

        // Unit type: ()
        if self.stream.check(&TokenKind::LParen)
            && let Some(unit) = self.optional(|p| {
                p.stream.expect(TokenKind::LParen)?;
                p.stream.expect(TokenKind::RParen)?;
                let span = p.stream.make_span(start_pos);
                Ok(Type::unit(span))
            })
        {
            return Ok(unit);
        }

        // Never type: ! (for diverging functions)
        if self.stream.check(&TokenKind::Bang) {
            self.stream.advance();
            let span = self.stream.make_span(start_pos);
            return Ok(Type::never(span));
        }

        // Unknown type: unknown (top type for safe dynamic typing)
        // Unknown type: top type for safe dynamic typing — all types are subtypes of unknown
        if self.stream.check(&TokenKind::Unknown) {
            self.stream.advance();
            let span = self.stream.make_span(start_pos);
            return Ok(Type::unknown(span));
        }

        // Try primitives
        if let Some(prim) = self.optional(|p| p.parse_primitive_type()) {
            return Ok(prim);
        }

        // Protocol types: impl Display + Debug, dyn Display + Debug
        // Note: `impl` is a contextual keyword (lexed as Ident), not TokenKind::Implement
        if self.stream.check(&TokenKind::Implement) {
            return self.parse_protocol_type();
        }

        if self.is_ident()
            && let Some(Token {
                kind: TokenKind::Ident(name),
                ..
            }) = self.stream.peek()
        {
            if name.as_str() == "impl" {
                return self.parse_protocol_type();
            }
            if name.as_str() == "dyn" {
                return self.parse_protocol_type();
            }
            if name.as_str() == "GenRef" {
                return self.parse_genref_type();
            }
            // Existential type: some T: Bound or some T: Bound1 + Bound2
            // Existential type: `some T: Bound` — opaque type implementing given protocol bounds
            if name.as_str() == "some" {
                return self.parse_existential_type();
            }
            // Path type: Path<A>(a, b) — propositional equality path.
            // Grammar: path_type_expr = 'Path' , type_args , '(' , expression , ',' , expression , ')' ;
            // `Path` is NOT a keyword — it's a context-sensitive identifier,
            // matching Verum's philosophy of 3 reserved keywords only.
            if name.as_str() == "Path" {
                if let Some(TokenKind::Lt) = self.stream.peek_nth(1).map(|t| t.kind.clone()) {
                    return self.parse_path_type_expr(start_pos);
                }
            }

            // Universe type: Type, Type(0), Type(1), Type(u)
            // Grammar: universe_type = 'Type' , [ '(' , universe_level , ')' ] ;
            // `Type` alone is Type(0), `Type(N)` is a concrete universe level,
            // `Type(u)` is a universe level variable for universe polymorphism.
            //
            // We parse `Type` as a universe type when:
            // - It's `Type` followed by `(` (explicit level annotation)
            // - It's `Type` NOT followed by `<` or `.` (bare Type = Type(0))
            // If followed by `<` or `.`, it falls through to path_type for backward compat.
            if name.as_str() == "Type" {
                // Check what follows: `Type(...)` or bare `Type`
                let next = self.stream.peek_nth(1).map(|t| t.kind.clone());
                match next {
                    // Type(N) or Type(u) — universe with explicit level
                    Some(TokenKind::LParen) => {
                        return self.parse_universe_type();
                    }
                    // Type<...> — generic path type, fall through
                    Some(TokenKind::Lt) => {}
                    // Type.Foo — associated/path type, fall through
                    Some(TokenKind::Dot) => {}
                    // Bare `Type` — universe Type(0)
                    _ => {
                        return self.parse_universe_type();
                    }
                }
            }
        }

        // Async function type: async fn(A, B) -> C
        // Grammar: function_type = [ 'async' ] , 'fn' , '(' , type_list , ')' , [ '->' , type_expr ] , [ context_clause ] ;
        // Transforms to: fn(A, B) -> Future<C>
        // Also handles async protocol bounds: async FnOnce(), async Iterator, etc.
        if self.stream.check(&TokenKind::Async) {
            // Look ahead: if next is 'fn', parse as async function type
            // Otherwise, treat 'async' as a modifier on a protocol bound (async FnOnce, etc.)
            if matches!(self.stream.peek_nth(1).map(|t| &t.kind), Some(TokenKind::Fn)) {
                return self.parse_async_function_type();
            } else if matches!(self.stream.peek_nth(1).map(|t| &t.kind), Some(TokenKind::Async)) {
                // E018: Duplicate async keyword is not allowed
                return Err(ParseError::invalid_syntax(
                    "duplicate `async` keyword is not allowed",
                    self.stream.current_span(),
                ));
            } else {
                // async TraitName(...) - skip 'async' modifier and parse as regular type
                // The async qualifier is treated as an annotation on the bound
                // e.g., `async FnOnce()` is parsed as `FnOnce()` with async semantics
                self.stream.advance(); // consume 'async'
                return self.parse_type_no_refinement();
            }
        }

        // Function type: fn(A, B) -> C or extern "C" fn(A, B) -> C
        if self.stream.check(&TokenKind::Fn) {
            return self.parse_function_type(Maybe::None);
        }

        // Extern function pointer type: extern "C" fn(A, B) -> C
        if self.stream.check(&TokenKind::Extern) {
            return self.parse_extern_function_type();
        }

        // Tuple type: (A, B, C) - at least 2 elements
        if self.stream.check(&TokenKind::LParen)
            && let Some(tuple) = self.optional(|p| p.parse_tuple_type())
        {
            return Ok(tuple);
        }

        // Array or slice type: [T; N] or [T]
        if self.stream.check(&TokenKind::LBracket) {
            return self.parse_array_or_slice_type();
        }

        // Reference types: &T, &mut T, &checked T, &unsafe T, %T, *const T, *mut T
        // Also check for StarStar to handle double pointers: **T
        // Also check for AmpersandAmpersand to handle double references: &&T
        if self.stream.check_any(&[
            TokenKind::Ampersand,
            TokenKind::Percent,
            TokenKind::Star,
            TokenKind::StarStar,
            TokenKind::AmpersandAmpersand,
        ]) {
            return self.parse_reference_type();
        }

        // Type placeholder: _ (must be checked BEFORE path-based types)
        if let Some(Token {
            kind: TokenKind::Ident(name),
            ..
        }) = self.stream.peek()
            && name.as_str() == "_"
        {
            let span = self.stream.current_span();
            self.stream.advance();
            return Ok(Type::inferred(span));
        }

        // Dependent type meta prefix: meta T, meta Nat, meta K
        // Used in dependent type contexts for compile-time type-level values.
        // Example: fn take<T>(k: meta K) -> SizedList<T, K>
        if self.stream.check(&TokenKind::Meta) {
            self.stream.advance(); // consume 'meta'
            let inner = self.parse_type_no_refinement()?;
            let span = self.stream.make_span(start_pos);
            return Ok(Type::new(TypeKind::Meta { inner: Heap::new(inner) }, span));
        }

        // Meta type: @Expr, @TokenStream, @Type, etc.
        // Used in staged metaprogramming: meta fn foo() -> @Expr { ... }
        // The @ prefix indicates a compile-time meta type.
        if self.stream.check(&TokenKind::At) {
            return self.parse_meta_type();
        }

        // Anonymous record type: { field: Type, ... }
        // Grammar: record_type = '{' , field_list , '}' ;
        // Note: This must be checked BEFORE path types to handle anonymous records
        if self.stream.check(&TokenKind::LBrace) {
            return self.parse_record_type();
        }

        // Path-based type (includes generics): List<T>, std::collections::Map
        // Note: SelfValue is `self` (value), SelfType is `Self` (type)
        // Also support super, crate, and module as path roots
        if self.is_ident()
            || self.stream.check(&TokenKind::SelfValue)
            || self.stream.check(&TokenKind::SelfType)
            || self.stream.check(&TokenKind::Super)
            || self.stream.check(&TokenKind::Cog)
            || self.stream.check(&TokenKind::Module)
        {
            return self.parse_path_type();
        }

        let span = self.stream.current_span();
        Err(ParseError::invalid_syntax("expected type", span))
    }

    /// Parse type lambda: |x| TypeExpr or |x, y| TypeExpr
    /// Used in dependent type contexts: Sigma<A, |a| B(a)>
    /// Grammar: '|' ident {',' ident} '|' type_expr
    fn parse_type_lambda(&mut self) -> ParseResult<Type> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Pipe)?;

        // Parse parameter names
        let mut params = Vec::new();
        if !self.stream.check(&TokenKind::Pipe) {
            let name = self.consume_ident()?;
            let name_span = self.stream.current_span();
            params.push(Ident::new(name, name_span));

            while self.stream.consume(&TokenKind::Comma).is_some() {
                let name = self.consume_ident()?;
                let name_span = self.stream.current_span();
                params.push(Ident::new(name, name_span));
            }
        }

        self.stream.expect(TokenKind::Pipe)?;

        // Parse body type
        let body = self.parse_type_no_refinement()?;

        let span = self.stream.make_span(start_pos);
        Ok(Type::new(
            TypeKind::TypeLambda {
                params: params.into_iter().collect(),
                body: Heap::new(body),
            },
            span,
        ))
    }

    /// Parse meta type: @Expr, @TokenStream, @Type, etc.
    /// Used in staged metaprogramming for compile-time type representations.
    /// Grammar: '@' identifier
    fn parse_meta_type(&mut self) -> ParseResult<Type> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::At)?;

        // Parse the meta type name
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        let span = self.stream.make_span(start_pos);

        // Construct as a path type with @ prefix
        // The name becomes a path like "@Expr" which the type checker treats as a meta type
        let segment = PathSegment::Name(Ident::new(
            Text::from(format!("@{}", name)),
            name_span,
        ));
        let path = Path::new(List::from(vec![segment]), span);

        Ok(Type::new(TypeKind::Path(path), span))
    }

    /// Parse anonymous record type: { x: Int, y: Int }
    /// Grammar: record_type = '{' , field_list , '}' ;
    ///
    /// Note: Anonymous record types in type expressions don't support visibility
    /// modifiers or attributes - those are only for named type definitions.
    fn parse_record_type(&mut self) -> ParseResult<Type> {
        use verum_ast::decl::{RecordField, Visibility};

        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::LBrace)?;

        let mut fields = List::new();

        // Parse field list: identifier ':' type_expr { ',' identifier ':' type_expr } [ ',' ]
        while !self.stream.check(&TokenKind::RBrace) {
            let field_start = self.stream.position();

            // Parse field name
            let name = self.consume_ident_or_keyword()?;
            let name_span = self.stream.current_span();

            // Expect colon
            self.stream.expect(TokenKind::Colon)?;

            // Parse field type
            let ty = self.parse_type()?;

            // Parse optional default value
            let default_value = if self.stream.consume(&TokenKind::Eq).is_some() {
                Maybe::Some(self.parse_expr()?)
            } else {
                Maybe::None
            };

            let field_span = self.stream.make_span(field_start);
            fields.push(RecordField {
                visibility: Visibility::Private,
                name: Ident::new(name, name_span),
                ty,
                attributes: List::new(),
                default_value,
                bit_spec: Maybe::None,
                span: field_span,
            });

            // Consume comma or semicolon as field separator
            // Both are accepted: `{ x: Int, y: Int }` or `{ x: Int; y: Int; }`
            if self.stream.consume(&TokenKind::Comma).is_none()
                && self.stream.consume(&TokenKind::Semicolon).is_none()
            {
                break;
            }
        }

        self.stream.expect(TokenKind::RBrace)?;

        let span = self.stream.make_span(start_pos);
        Ok(Type::new(TypeKind::Record { fields }, span))
    }

    /// Parse universe type: `Type`, `Type(0)`, `Type(1)`, `Type(u)`.
    ///
    /// Grammar: universe_type = 'Type' , [ '(' , universe_level , ')' ] ;
    /// universe_level = integer_lit | identifier ;
    ///
    /// Bare `Type` is equivalent to `Type(0)` (the base universe).
    /// `Type(N)` for integer N is a concrete universe level.
    /// `Type(u)` for identifier u is a universe level variable (universe polymorphism).
    fn parse_universe_type(&mut self) -> ParseResult<Type> {
        let start_pos = self.stream.position();

        // Consume 'Type' identifier
        let name = self.consume_ident()?;
        debug_assert_eq!(name.as_str(), "Type");

        // Check for explicit level annotation: Type(...)
        let level = if self.stream.consume(&TokenKind::LParen).is_some() {
            let level_expr = self.parse_universe_level_expr()?;
            self.stream.expect(TokenKind::RParen)?;
            Maybe::Some(level_expr)
        } else {
            // Bare `Type` -- implicitly Type(0)
            Maybe::None
        };

        let span = self.stream.make_span(start_pos);
        Ok(Type::new(TypeKind::Universe { level }, span))
    }

    /// Parse a universe level expression inside `Type(...)`.
    ///
    /// Grammar:
    ///   universe_level_expr = integer_lit
    ///                       | identifier
    ///                       | 'max' '(' universe_level_expr ',' universe_level_expr ')'
    ///
    /// Examples:
    ///   Type(0)          -- concrete level 0
    ///   Type(u)          -- level variable u
    ///   Type(max(u, v))  -- maximum of two levels
    fn parse_universe_level_expr(&mut self) -> ParseResult<UniverseLevelExpr> {
        match self.stream.peek() {
            // Concrete level: 0, 1, 2, ...
            Some(Token {
                kind: TokenKind::Integer(int_lit),
                ..
            }) => {
                let val = int_lit
                    .as_i64()
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or_else(|| {
                        ParseError::invalid_syntax(
                            "universe level must be a non-negative integer",
                            self.stream.current_span(),
                        )
                    })?;
                self.stream.advance();
                Ok(UniverseLevelExpr::Concrete(val))
            }
            // max(u, v) -- maximum of two universe levels
            Some(Token {
                kind: TokenKind::Ident(kw),
                ..
            }) if kw.as_str() == "max" => {
                self.stream.advance(); // consume 'max'
                self.stream.expect(TokenKind::LParen)?;
                let lhs = self.parse_universe_level_expr()?;
                self.stream.expect(TokenKind::Comma)?;
                let rhs = self.parse_universe_level_expr()?;
                self.stream.expect(TokenKind::RParen)?;
                Ok(UniverseLevelExpr::Max(
                    Heap::new(lhs),
                    Heap::new(rhs),
                ))
            }
            // Level variable: u, v, ...
            Some(Token {
                kind: TokenKind::Ident(_),
                ..
            }) => {
                let var_name = self.consume_ident()?;
                let var_span = self.stream.current_span();
                Ok(UniverseLevelExpr::Variable(Ident::new(var_name, var_span)))
            }
            _ => Err(ParseError::invalid_syntax(
                "expected universe level expression (integer, level variable, or max(u, v))",
                self.stream.current_span(),
            )),
        }
    }

    /// Parse path type expression: `Path<A>(a, b)`.
    /// Grammar: `path_type_expr = 'Path' , type_args , '(' , expression , ',' , expression , ')' ;`
    fn parse_path_type_expr(&mut self, start_pos: usize) -> ParseResult<Type> {
        // Consume 'Path' identifier
        let name = self.consume_ident()?;
        debug_assert_eq!(name.as_str(), "Path");

        // Parse type arguments: <A>
        self.stream.expect(TokenKind::Lt)?;
        let carrier = self.parse_type()?;
        self.stream.expect(TokenKind::Gt)?;

        // Parse endpoint expressions: (a, b) with optional trailing comma.
        // Trailing commas are a universal convention in Verum (tuples,
        // record literals, argument lists), and the stdlib relies on
        // them for multi-line Path<...>(a, b,) layouts in e.g.
        // `core/math/hott.vr`. Rejecting the trailing comma here would
        // force the stdlib into a one-line style or introduce a pointless
        // stylistic exception.
        self.stream.expect(TokenKind::LParen)?;
        let lhs = self.parse_expr_no_struct()?;
        self.stream.expect(TokenKind::Comma)?;
        let rhs = self.parse_expr_no_struct()?;
        self.stream.consume(&TokenKind::Comma);
        self.stream.expect(TokenKind::RParen)?;

        let span = self.stream.make_span(start_pos);
        Ok(Type::new(
            TypeKind::PathType {
                carrier: verum_common::Heap::new(carrier),
                lhs: verum_common::Heap::new(lhs),
                rhs: verum_common::Heap::new(rhs),
            },
            span,
        ))
    }

    /// Parse primitive types: Bool, Int, Float, Char, Text
    fn parse_primitive_type(&mut self) -> ParseResult<Type> {
        match self.stream.peek() {
            Some(Token {
                kind: TokenKind::Ident(name),
                span,
            }) if name.as_str() == "Bool" => {
                let span = *span;
                self.stream.advance();
                Ok(Type::bool(span))
            }
            Some(Token {
                kind: TokenKind::Ident(name),
                span,
            }) if name.as_str() == "Int" => {
                let span = *span;
                self.stream.advance();
                Ok(Type::int(span))
            }
            Some(Token {
                kind: TokenKind::Ident(name),
                span,
            }) if name.as_str() == "Float" => {
                let span = *span;
                self.stream.advance();
                Ok(Type::float(span))
            }
            Some(Token {
                kind: TokenKind::Ident(name),
                span,
            }) if name.as_str() == "Char" => {
                let span = *span;
                self.stream.advance();
                Ok(Type::new(TypeKind::Char, span))
            }
            Some(Token {
                kind: TokenKind::Ident(name),
                span,
            }) if name.as_str() == "Text" => {
                let span = *span;
                self.stream.advance();
                Ok(Type::text(span))
            }
            _ => {
                let span = self.stream.current_span();
                Err(ParseError::invalid_syntax("expected primitive type", span))
            }
        }
    }

    /// Parse GenRef type: GenRef<T>
    /// GenRef<T> is a generation-aware reference used in CBGR's lending iterator pattern.
    /// GATs in CBGR use generation tracking instead of lifetime annotations. GenRef wraps
    /// a reference with automatic generation validity checking (~15ns overhead).
    fn parse_genref_type(&mut self) -> ParseResult<Type> {
        let start_pos = self.stream.position();

        // Consume "GenRef"
        if let Some(Token {
            kind: TokenKind::Ident(name),
            ..
        }) = self.stream.peek()
            && name.as_str() != "GenRef"
        {
            return Err(ParseError::invalid_syntax(
                "expected GenRef",
                self.stream.current_span(),
            ));
        }
        self.stream.advance();

        // Parse <T>
        self.stream.expect(TokenKind::Lt)?;
        let inner = self.parse_type()?;
        self.expect_gt()?;

        let span = self.stream.make_span(start_pos);
        Ok(Type::new(
            TypeKind::GenRef {
                inner: Box::new(inner),
            },
            span,
        ))
    }

    /// Parse existential type: some T: Bound or some T: Bound1 + Bound2
    /// Existential types represent opaque types implementing certain protocol bounds.
    /// Used for return type abstraction and type erasure: `some T: Bound1 + Bound2`.
    ///
    /// Existential types represent opaque types that implement certain protocols.
    /// They are used for return type abstraction (hiding implementation details)
    /// and for type erasure.
    ///
    /// Examples:
    /// - `some T: Iterator` - some type implementing Iterator
    /// - `some T: Display + Debug` - some type implementing both Display and Debug
    /// - `some I: Iterator where I.Item = Int` - some Iterator yielding Int
    fn parse_existential_type(&mut self) -> ParseResult<Type> {
        let start_pos = self.stream.position();

        // Consume "some"
        if let Some(Token {
            kind: TokenKind::Ident(name),
            ..
        }) = self.stream.peek()
            && name.as_str() != "some"
        {
            return Err(ParseError::invalid_syntax(
                "expected 'some'",
                self.stream.current_span(),
            ));
        }
        self.stream.advance();

        // Parse the type parameter name: T
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();
        let name_ident = Ident::new(name, name_span);

        // E098: Check for missing colon - if next is an identifier, it should be preceded by colon
        if !self.stream.check(&TokenKind::Colon) {
            if self.is_ident() || self.stream.check(&TokenKind::LBrace) {
                return Err(ParseError::existential_missing_colon(self.stream.make_span(start_pos)));
            }
        }

        // Expect colon followed by bounds: : Bound1 + Bound2
        self.stream.expect(TokenKind::Colon)?;

        // E097: Check for empty bounds - `some T: {}` or `some T: )`
        if self.stream.check(&TokenKind::LBrace) || self.stream.check(&TokenKind::RParen) {
            return Err(ParseError::existential_no_bounds(self.stream.make_span(start_pos)));
        }

        // Parse bounds using parse_type_bounds_or_type to support generic bounds
        // like Iterator<Item = Int>, not just simple paths
        let bounds = self.parse_type_bounds_or_type()?;

        let span = self.stream.make_span(start_pos);
        Ok(Type::new(
            TypeKind::Existential {
                name: name_ident,
                bounds,
            },
            span,
        ))
    }

    /// Parse extern function pointer type: extern "C" fn(A, B) -> C
    ///
    /// Syntax:
    /// - `extern fn(...)` - defaults to C calling convention
    /// - `extern "C" fn(...)` - explicit C calling convention
    /// - `extern "stdcall" fn(...)` - Windows stdcall
    /// - `extern "fastcall" fn(...)` - Fast call convention
    /// - `extern "sysv64" fn(...)` - System V AMD64 ABI
    fn parse_extern_function_type(&mut self) -> ParseResult<Type> {
        let start_pos = self.stream.position();

        self.stream.expect(TokenKind::Extern)?;

        // Parse optional ABI string: "C", "stdcall", "fastcall", "sysv64"
        let calling_convention = if let Some(TokenKind::Text(abi)) = self.stream.peek_kind() {
            let abi_str = abi.clone();
            self.stream.advance(); // consume the string
            match abi_str.as_str() {
                "C" => CallingConvention::C,
                "stdcall" => CallingConvention::StdCall,
                "fastcall" => CallingConvention::FastCall,
                "sysv64" => CallingConvention::SysV64,
                "system" => CallingConvention::System,
                "interrupt" => CallingConvention::Interrupt,
                "naked" => CallingConvention::Naked,
                _ => {
                    return Err(ParseError::invalid_syntax(
                        format!("unknown calling convention: \"{}\". Expected \"C\", \"stdcall\", \"fastcall\", \"sysv64\", or \"system\"", abi_str),
                        self.stream.current_span(),
                    ));
                }
            }
        } else {
            // Default to C calling convention if no ABI string
            CallingConvention::C
        };

        // Now parse the function type with the calling convention
        self.parse_function_type(Maybe::Some(calling_convention))
    }

    /// Parse function type: fn(A, B) -> C or fn<R>(A, R) -> R (rank-2)
    ///
    /// The `calling_convention` parameter is `Some` when parsing extern function
    /// pointer types (e.g., `extern "C" fn(...)`), `None` for regular function types.
    ///
    /// Rank-2 function types have universally quantified type parameters:
    /// - `fn<R>(Reducer<B, R>) -> Reducer<A, R>` - R is quantified within the function type
    /// - This enables storing polymorphic functions as values (e.g., transducers)
    fn parse_function_type(&mut self, calling_convention: Maybe<CallingConvention>) -> ParseResult<Type> {
        let start_pos = self.stream.position();

        self.stream.expect(TokenKind::Fn)?;

        // Check for rank-2 type parameters: fn<R, S: Clone>(...)
        let type_params = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?
        } else {
            List::new()
        };

        // Parse parameter types: (A, B, C)
        self.stream.expect(TokenKind::LParen)?;
        let params = if self.stream.check(&TokenKind::RParen) {
            Vec::new()
        } else {
            self.comma_separated(|p| p.parse_type())?
        };
        // E061: Unclosed function parameter list
        if self.stream.consume(&TokenKind::RParen).is_none() {
            return Err(ParseError::unclosed_fn_params(self.stream.make_span(start_pos)));
        }

        // Optional throws clause: throws(ErrorType) or throws(A | B)
        // throws MUST be followed by parenthesized error type
        if self.stream.check(&TokenKind::Throws) {
            self.stream.advance(); // consume 'throws'
            if self.stream.consume(&TokenKind::LParen).is_some() {
                // Parse error types: Type | Type | ...
                self.parse_type()?;
                while self.stream.consume(&TokenKind::Pipe).is_some() {
                    self.parse_type()?;
                }
                self.stream.expect(TokenKind::RParen)?;
            } else {
                return Err(ParseError::invalid_syntax(
                    "`throws` must be followed by a parenthesized error type, e.g., `throws(Error)`",
                    self.stream.current_span(),
                ));
            }
        }

        // E063: Wrong arrow operator (=> instead of ->)
        if self.stream.check(&TokenKind::FatArrow) {
            return Err(ParseError::with_error_code(
                ParseErrorKind::InvalidSyntax { message: "function type must use `->` not `=>`".into() },
                self.stream.current_span(),
                ErrorCode::WrongArrowOperator,
            ));
        }

        // Parse return type: -> C
        // Use parse_type_with_lookahead to avoid consuming function body { as a refinement
        let return_type = if self.stream.consume(&TokenKind::RArrow).is_some() {
            // E062: Missing return type after arrow
            if matches!(
                self.stream.peek_kind(),
                Some(TokenKind::Semicolon) | Some(TokenKind::RBrace) | Some(TokenKind::RBracket) | None
            ) {
                return Err(ParseError::with_error_code(
                    ParseErrorKind::InvalidSyntax { message: "missing return type after `->`".into() },
                    self.stream.make_span(start_pos),
                    ErrorCode::FnTypeMissingReturn,
                ));
            }
            self.parse_type_with_lookahead()?
        } else {
            let span = self.stream.current_span();
            Type::unit(span)
        };

        // Optional context clause AFTER return type (CANONICAL FORM)
        // Context clause: `using [Ctx1, Ctx2]` or `using Ctx` after return type (canonical form)
        // Grammar: context_clause = 'using' , context_spec ;
        // Format: fn(A) -> B using [Context]          -- bracketed contexts
        // Format: fn(A) -> B using Context            -- single context
        // Format: fn(A) using [Context]               -- when return type is unit
        // Supports all advanced patterns: negative (!), alias (as), named (:),
        // conditional (if), transforms (.), type arguments (<>)
        let contexts: ContextList = if self.stream.consume(&TokenKind::Using).is_some() {
            self.parse_using_contexts()?.into()
        } else {
            ContextList::empty()
        };

        // Optional where clause for rank-2 types: fn<T>(T) -> T where type T: Clone
        let where_clause = if !type_params.is_empty() && self.stream.check(&TokenKind::Where) {
            Maybe::Some(self.parse_where_clause()?)
        } else {
            Maybe::None
        };

        let span = self.stream.make_span(start_pos);

        // Create Rank2Function if we have type parameters, otherwise regular Function
        let kind = if type_params.is_empty() {
            TypeKind::Function {
                params: params.into_iter().collect::<List<_>>(),
                return_type: Box::new(return_type),
                calling_convention,
                contexts,
            }
        } else {
            TypeKind::Rank2Function {
                type_params,
                params: params.into_iter().collect::<List<_>>(),
                return_type: Box::new(return_type),
                calling_convention,
                contexts,
                where_clause,
            }
        };

        Ok(Type::new(kind, span))
    }

    /// Parse async function type: async fn(A, B) -> C
    /// Transforms to fn(A, B) -> Future<C>
    ///
    /// Grammar: function_type = [ 'async' ] , 'fn' , '(' , type_list , ')' , [ '->' , type_expr ] , [ context_clause ] ;
    fn parse_async_function_type(&mut self) -> ParseResult<Type> {
        let start_pos = self.stream.position();

        // Consume 'async'
        self.stream.expect(TokenKind::Async)?;

        // Parse the rest as a regular function type
        let fn_type = self.parse_function_type(Maybe::None)?;

        // Wrap the return type in Future<T>
        // async fn() -> T becomes fn() -> Future<T>
        // async fn() becomes fn() -> Future<()>
        match fn_type.kind {
            TypeKind::Function {
                params,
                return_type,
                calling_convention,
                contexts,
            } => {
                // Create Future<return_type>
                // First create the base path type for "Future"
                let future_span = self.stream.make_span(start_pos);
                let future_ident = Ident::new("Future", future_span);
                let future_path = Path::single(future_ident);
                let future_base = Type::new(TypeKind::Path(future_path), future_span);

                // Then wrap in Generic with the return type as argument
                let future_type = Type::new(
                    TypeKind::Generic {
                        base: Box::new(future_base),
                        args: List::from(vec![GenericArg::Type(*return_type)]),
                    },
                    future_span,
                );

                let span = self.stream.make_span(start_pos);
                Ok(Type::new(
                    TypeKind::Function {
                        params,
                        return_type: Box::new(future_type),
                        calling_convention,
                        contexts,
                    },
                    span,
                ))
            }
            TypeKind::Rank2Function {
                type_params,
                params,
                return_type,
                calling_convention,
                contexts,
                where_clause,
            } => {
                // Create Future<return_type>
                // First create the base path type for "Future"
                let future_span = self.stream.make_span(start_pos);
                let future_ident = Ident::new("Future", future_span);
                let future_path = Path::single(future_ident);
                let future_base = Type::new(TypeKind::Path(future_path), future_span);

                // Then wrap in Generic with the return type as argument
                let future_type = Type::new(
                    TypeKind::Generic {
                        base: Box::new(future_base),
                        args: List::from(vec![GenericArg::Type(*return_type)]),
                    },
                    future_span,
                );

                let span = self.stream.make_span(start_pos);
                Ok(Type::new(
                    TypeKind::Rank2Function {
                        type_params,
                        params,
                        return_type: Box::new(future_type),
                        calling_convention,
                        contexts,
                        where_clause,
                    },
                    span,
                ))
            }
            _ => {
                // Should not happen - parse_function_type always returns Function or Rank2Function
                let span = self.stream.make_span(start_pos);
                Err(ParseError::invalid_syntax(
                    "internal error: expected function type",
                    span,
                ))
            }
        }
    }

    /// Parse tuple type: (A, B, C) - requires at least 2 elements
    fn parse_tuple_type(&mut self) -> ParseResult<Type> {
        let start_pos = self.stream.position();

        self.stream.expect(TokenKind::LParen)?;

        let types = if self.stream.check(&TokenKind::RParen) {
            Vec::new()
        } else {
            self.comma_separated(|p| p.parse_type())?
        };

        self.stream.expect(TokenKind::RParen)?;

        let span = self.stream.make_span(start_pos);

        // Single element is just a parenthesized type, not a tuple
        // Tuples need at least 2 elements
        if types.len() == 1 {
            // Just return the inner type (parentheses are ignored in type syntax)
            // SAFETY: len() == 1 guarantees exactly one element
            return Ok(types.into_iter().next()
                .expect("types.len() == 1 guarantees one element"));
        }

        Ok(Type::new(
            TypeKind::Tuple(types.into_iter().collect::<List<_>>()),
            span,
        ))
    }

    /// Parse array or slice type: [T; N] or [T]
    fn parse_array_or_slice_type(&mut self) -> ParseResult<Type> {
        let start_pos = self.stream.position();

        self.stream.expect(TokenKind::LBracket)?;

        // E074: Array missing element type - [; 10] or []
        if self.stream.check(&TokenKind::Semicolon) || self.stream.check(&TokenKind::RBracket) {
            return Err(ParseError::array_missing_element(self.stream.make_span(start_pos)));
        }

        let element = self.parse_type()?;

        // E073: Missing semicolon before size - [T N] instead of [T; N]
        // If we see an integer directly after the type (no semicolon), that's an error
        if matches!(self.stream.peek_kind(), Some(TokenKind::Integer(_))) {
            return Err(ParseError::array_double_semicolon(self.stream.make_span(start_pos)));
        }

        let result = if self.stream.consume(&TokenKind::Semicolon).is_some() {
            // E073: Array double semicolon - [T;; N]
            if self.stream.check(&TokenKind::Semicolon) {
                return Err(ParseError::array_double_semicolon(self.stream.make_span(start_pos)));
            }
            // E071: Array missing size - [T;] no size after semicolon
            if self.stream.check(&TokenKind::RBracket) {
                return Err(ParseError::array_missing_size(self.stream.make_span(start_pos)));
            }
            // Array type: [T; N]
            let size = self.parse_expr()?;
            // E070: Unclosed array type - [T; N) missing ]
            if self.stream.consume(&TokenKind::RBracket).is_none() {
                return Err(ParseError::unclosed_array_type(self.stream.make_span(start_pos)));
            }

            let span = self.stream.make_span(start_pos);
            Type::new(
                TypeKind::Array {
                    element: Box::new(element),
                    size: Some(Box::new(size)),
                },
                span,
            )
        } else {
            // Slice type: [T]
            // E070: Unclosed array/slice type
            if self.stream.consume(&TokenKind::RBracket).is_none() {
                return Err(ParseError::unclosed_array_type(self.stream.make_span(start_pos)));
            }

            let span = self.stream.make_span(start_pos);
            Type::new(TypeKind::Slice(Box::new(element)), span)
        };

        Ok(result)
    }

    /// Parse reference types: &T, &mut T, &checked T, &unsafe T, &'a T, %T, *const T, *mut T
    /// Also handles && (AmpersandAmpersand) by splitting it into two & tokens for double references: &&T
    fn parse_reference_type(&mut self) -> ParseResult<Type> {
        let start_pos = self.stream.position();

        // Handle && (AmpersandAmpersand) token by splitting it into two & tokens
        // This allows us to parse &&T as &(&T)
        // Track whether we came from a pending split (for E056 check below)
        let from_pending_split = self.pending_ampersand;
        let has_ampersand = if self.pending_ampersand {
            // Consume the pending & from a previous && split
            self.pending_ampersand = false;
            // We already "consumed" one & from the &&, now we need to actually advance past the && token
            // But only if we haven't already advanced past it
            if self.stream.peek_kind() == Some(&TokenKind::AmpersandAmpersand) {
                self.stream.advance();
            }
            true
        } else if self.stream.consume(&TokenKind::Ampersand).is_some() {
            true
        } else if self.stream.peek_kind() == Some(&TokenKind::AmpersandAmpersand) {
            // Split && into two & tokens
            // First & is "consumed" now, second & will be consumed when pending_ampersand is processed
            self.pending_ampersand = true;
            // Don't advance! Keep the && token in the stream for now
            true
        } else {
            false
        };

        // CBGR references: &T, &mut T, &checked T, &unsafe T, &'a T
        if has_ampersand {
            // E056: Double ampersand with space `& &T`
            // Skip this check if we came from a pending split (e.g., parsing &&&T)
            // because in that case the following & is expected for the third reference level
            if !from_pending_split && self.stream.check(&TokenKind::Ampersand) {
                return Err(ParseError::double_ampersand_ref(self.stream.make_span(start_pos)));
            }

            // E057: Reference without type `&)` or `&;` or `&,`
            if matches!(
                self.stream.peek_kind(),
                Some(TokenKind::RParen) | Some(TokenKind::Semicolon) | Some(TokenKind::Comma) | Some(TokenKind::RBracket)
            ) {
                return Err(ParseError::ref_without_type(self.stream.make_span(start_pos)));
            }

            // Optional lifetime annotation: &'a T
            // Note: Currently the AST doesn't store lifetime in Reference types,
            // but we parse it for forward compatibility
            let _lifetime = if matches!(self.stream.peek_kind(), Some(TokenKind::Lifetime(_))) {
                Some(self.parse_lifetime()?)
            } else {
                None
            };

            // Check for &checked or &unsafe
            let is_checked = self.stream.consume(&TokenKind::Checked).is_some();
            let is_unsafe = if !is_checked {
                self.stream.consume(&TokenKind::Unsafe).is_some()
            } else {
                false
            };

            // E058: Double checked `&checked checked T` or double unsafe
            if is_checked && self.stream.check(&TokenKind::Checked) {
                return Err(ParseError::double_checked_ref(self.stream.make_span(start_pos)));
            }
            if is_unsafe && self.stream.check(&TokenKind::Unsafe) {
                return Err(ParseError::double_checked_ref(self.stream.make_span(start_pos)));
            }

            // E059: Conflicting modifiers `&checked unsafe T`
            if is_checked && self.stream.check(&TokenKind::Unsafe) {
                return Err(ParseError::conflicting_ref_modifiers(self.stream.make_span(start_pos)));
            }
            if is_unsafe && self.stream.check(&TokenKind::Checked) {
                return Err(ParseError::conflicting_ref_modifiers(self.stream.make_span(start_pos)));
            }

            // Check for mut
            let mutable = self.stream.consume(&TokenKind::Mut).is_some();

            // E058: Double mut `&mut mut T`
            if mutable && self.stream.check(&TokenKind::Mut) {
                return Err(ParseError::double_checked_ref(self.stream.make_span(start_pos)));
            }

            // Parse inner type WITHOUT refinements
            // Refinements on references are parsed at the outer level, not inside the reference.
            // This prevents the parser from consuming function body braces as refinements:
            // where F: fn(Int) -> Int { body }
            // Without this, the parser would try to parse `Int{ body }` as a refinement.
            // If you need &(Int{> 0}), write it explicitly with parentheses.
            let inner = self.parse_type_no_refinement()?;
            let span = self.stream.make_span(start_pos);

            let kind = if is_checked {
                TypeKind::CheckedReference {
                    mutable,
                    inner: Box::new(inner),
                }
            } else if is_unsafe {
                TypeKind::UnsafeReference {
                    mutable,
                    inner: Box::new(inner),
                }
            } else {
                TypeKind::Reference {
                    mutable,
                    inner: Box::new(inner),
                }
            };

            return Ok(Type::new(kind, span));
        }

        // Ownership reference: %T, %mut T
        if self.stream.consume(&TokenKind::Percent).is_some() {
            let mutable = self.stream.consume(&TokenKind::Mut).is_some();
            // Parse inner type WITHOUT refinements to avoid consuming function body brace
            let inner = self.parse_type_no_refinement()?;
            let span = self.stream.make_span(start_pos);

            return Ok(Type::new(
                TypeKind::Ownership {
                    mutable,
                    inner: Box::new(inner),
                },
                span,
            ));
        }

        // Raw pointer: *const T, *mut T, or bare *T (defaults to *mut for C FFI)
        // Also handle ** for double pointers by splitting StarStar token
        let has_star = if self.pending_star {
            // Consume the pending * from a previous StarStar split
            self.pending_star = false;
            if self.stream.peek_kind() == Some(&TokenKind::StarStar) {
                self.stream.advance();
            }
            true
        } else if self.stream.consume(&TokenKind::Star).is_some() {
            true
        } else if self.stream.peek_kind() == Some(&TokenKind::StarStar) {
            // Split ** into two * tokens
            self.pending_star = true;
            // Don't advance! Keep the ** token in the stream
            true
        } else {
            false
        };

        if has_star {
            // Check for volatile pointer: *volatile T or *volatile mut T
            if self.stream.consume(&TokenKind::Volatile).is_some() {
                let mutable = self.stream.consume(&TokenKind::Mut).is_some();
                let inner = self.parse_type_no_refinement()?;
                let span = self.stream.make_span(start_pos);
                return Ok(Type::new(
                    TypeKind::VolatilePointer {
                        mutable,
                        inner: Box::new(inner),
                    },
                    span,
                ));
            }

            let mutable = if self.stream.consume(&TokenKind::Mut).is_some() {
                true
            } else if self.stream.consume(&TokenKind::Const).is_some() {
                false
            } else {
                // Allow bare pointer syntax for C FFI: *void, *FILE
                // Defaults to mutable (*mut) for C compatibility
                true
            };

            // Parse inner type WITHOUT refinements to avoid consuming function body brace
            // For example: fn get_ptr() -> *const Int { body }
            // Without this, the parser would try to parse `Int{ body }` as a refinement.
            let inner = self.parse_type_no_refinement()?;
            let span = self.stream.make_span(start_pos);

            return Ok(Type::new(
                TypeKind::Pointer {
                    mutable,
                    inner: Box::new(inner),
                },
                span,
            ));
        }

        Err(ParseError::invalid_syntax(
            "expected reference type",
            self.stream.current_span(),
        ))
    }

    /// Parse path-based type with optional generic arguments.
    /// Also handles associated type instantiation: Protocol::Assoc<T> or Self.Item<T>
    fn parse_path_type(&mut self) -> ParseResult<Type> {
        let start_pos = self.stream.position();

        let path = self.parse_path()?;

        // Check for associated type projection with dot notation: Self.Item<T>, T.Item<U>
        // This is Verum's syntax for associated types (not Self::Item)
        // Associated type projection with dot notation: `Self.Item<T>`, `T.Assoc<U>`
        // GATs allow associated types to have their own type parameters (future release feature)

        // NOTE: Currently, ALL multi-segment paths are treated as qualified/associated types.
        // This works for `T.Item` (associated type) but incorrectly treats `module.Type`
        // (module path) as qualified. This is a known limitation that will be addressed
        // when we have better context (e.g., distinguish type parameters from module names).
        //
        // For chained associated types like C.Iter.Item, we need to create nested Qualified types:
        // C.Iter.Item becomes Qualified { self_ty: Qualified { self_ty: Path(C), assoc: Iter }, assoc: Item }
        let mut base_type = if path.segments.len() > 1 {
            // Path like T.Item or C.Iter.Item - recursively create nested Qualified types
            let segments_vec: Vec<_> = path.segments.iter().cloned().collect();

            // Start with the first segment as a Path type
            let first_segment = &segments_vec[0];
            let first_path = Path::new(List::from(vec![first_segment.clone()]), path.span);
            let mut current_type = Type::new(TypeKind::Path(first_path), path.span);

            // Iterate through remaining segments, creating nested Qualified types
            for segment in segments_vec.iter().skip(1) {
                // Convert segment to an Ident for the associated type name
                let assoc_ident = match segment {
                    PathSegment::Name(ident) => ident.clone(),
                    PathSegment::SelfValue => Ident::new("Self", path.span),
                    PathSegment::Super => Ident::new("super", path.span),
                    PathSegment::Cog => Ident::new("crate", path.span),
                    _ => {
                        return Err(ParseError::invalid_syntax(
                            "expected identifier for associated type name",
                            path.span,
                        ));
                    }
                };

                // Wrap current type in a Qualified type
                current_type = Type::new(
                    TypeKind::Qualified {
                        self_ty: Box::new(current_type),
                        trait_ref: Path::new(List::new(), path.span),
                        assoc_name: assoc_ident,
                    },
                    path.span,
                );
            }

            current_type
        } else {
            // Single segment path - just a regular path type
            Type::new(
                TypeKind::Path(path.clone()),
                self.stream.make_span(start_pos),
            )
        };

        // Handle additional dot notation for nested associated types: Self.Item.Output
        // This is less common but still valid
        //
        // IMPORTANT: Only consume `.` if followed by an identifier.
        // This prevents `.` from being consumed when it's used as a separator in
        // forall/exists expressions (e.g., `forall i: T . body`), where the `.`
        // separates the quantifier binding from the body expression.
        while self.stream.check(&TokenKind::Dot) && self.is_dot_followed_by_ident() {
            self.stream.advance(); // consume '.'
            // Parse the associated type name
            let assoc_name = self.consume_ident()?;
            let assoc_span = self.stream.current_span();
            let assoc_ident = Ident::new(assoc_name, assoc_span);

            // Create a Qualified type: Self.Item
            // Note: For Self.Item, the trait_ref is empty (we'll use the path itself)
            // This will be resolved during type checking
            base_type = Type::new(
                TypeKind::Qualified {
                    self_ty: Box::new(base_type),
                    trait_ref: Path::new(List::new(), assoc_span),
                    assoc_name: assoc_ident,
                },
                self.stream.make_span(start_pos),
            );
        }

        // E055: Detect double opening angle bracket like List<<Int>
        // The lexer tokenizes << as LtLt (left shift), so we need to check for it
        if self.stream.check(&TokenKind::LtLt) {
            return Err(ParseError::double_angle_bracket(self.stream.current_span()));
        }

        // Handle angle-bracketed generic arguments: <T, U>
        if self.stream.check(&TokenKind::Lt) {
            let args = self.parse_generic_args()?;
            let generic_span = self.stream.make_span(start_pos);

            let generic_ty = Type::new(
                TypeKind::Generic {
                    base: Box::new(base_type),
                    args: args.into_iter().collect::<List<_>>(),
                },
                generic_span,
            );

            // After `<…>`, check for value-argument parentheses:
            //   Fiber<A, B>(f, b)
            //   IsContrMap<A, B>(f)
            //   Glue<A>(phi, T, e)
            //
            // This is the general dependent-type application shape that
            // `core/math/hott.vr`, `cubical.vr`, `infinity_topos.vr`, and
            // `kan_extension.vr` pervasively rely on. The two-argument
            // `Path<A>(a, b)` form has its own special-cased entry earlier
            // (`parse_path_type_expr`) for historical compatibility; every
            // other type with value indices flows through here.
            if self.stream.check(&TokenKind::LParen) {
                self.stream.advance();
                let mut value_args: Vec<Expr> = Vec::new();
                if !self.stream.check(&TokenKind::RParen) {
                    loop {
                        if !self.tick() || self.is_aborted() {
                            break;
                        }
                        let expr = self.parse_expr_no_struct()?;
                        value_args.push(expr);
                        if self.stream.consume(&TokenKind::Comma).is_none() {
                            break;
                        }
                        // Allow trailing comma: `Foo<T>(a, b, )`.
                        if self.stream.check(&TokenKind::RParen) {
                            break;
                        }
                    }
                }
                self.stream.expect(TokenKind::RParen)?;
                let span = self.stream.make_span(start_pos);
                return Ok(Type::new(
                    TypeKind::DependentApp {
                        carrier: Box::new(generic_ty),
                        value_args: value_args.into_iter().collect::<List<_>>(),
                    },
                    span,
                ));
            }

            Ok(generic_ty)
        }
        // Handle parenthesized type arguments: (T, U)
        // This is used for constructor-style type applications like: A(B(C(Int)))
        // Also handles function trait syntax: Fn(T, U) -> R
        else if self.stream.check(&TokenKind::LParen) {
            self.stream.advance(); // consume (

            let args = if self.stream.check(&TokenKind::RParen) {
                Vec::new()
            } else {
                self.comma_separated(|p| {
                    // Use parse_type_no_refinement to avoid consuming { } as refinement
                    // This is crucial for function type bounds in where clauses:
                    // where F: fn(Int) -> Int { body }
                    // We don't want to consume the function body { } as a type refinement
                    let ty = p.parse_type_no_refinement()?;
                    Ok(GenericArg::Type(ty))
                })?
            };

            self.stream.expect(TokenKind::RParen)?;

            // Check for function trait return type syntax: -> R
            // This converts Fn(T, U) -> R into Fn<(T, U), Output = R>
            let mut final_args = args;
            if self.stream.consume(&TokenKind::RArrow).is_some() {
                // Use parse_type_no_refinement to avoid consuming { } as refinement
                // This is especially important in where clauses
                let return_type = self.parse_type_no_refinement()?;
                // Add Output = R as an associated type binding
                // For now, we'll represent this as a Const generic arg wrapping the type
                // The type checker will interpret this correctly
                final_args.push(GenericArg::Type(return_type));
            }

            let span = self.stream.make_span(start_pos);

            Ok(Type::new(
                TypeKind::Generic {
                    base: Box::new(base_type),
                    args: final_args.into_iter().collect::<List<_>>(),
                },
                span,
            ))
        } else {
            Ok(base_type)
        }
    }

    /// Parse generic arguments: <T, U, const N: usize, _>
    /// Supports Higher-Kinded Types (HKT) with _ placeholder
    pub(crate) fn parse_generic_args(&mut self) -> ParseResult<Vec<GenericArg>> {
        let start_pos = self.stream.position();

        // E055: Detect double opening angle bracket like List<<Int>
        // The lexer tokenizes << as LtLt (left shift), so we need to check for it
        if self.stream.check(&TokenKind::LtLt) {
            return Err(ParseError::double_angle_bracket(self.stream.current_span()));
        }

        self.stream.expect(TokenKind::Lt)?;

        // E055: Also detect when we have <$ followed by another <
        if self.stream.check(&TokenKind::Lt) {
            return Err(ParseError::double_angle_bracket(self.stream.current_span()));
        }

        // Grammar requires at least one type argument (verum.ebnf line 537):
        // type_args = '<' , type_arg , { ',' , type_arg } , '>' ;
        // Empty generics like Vec<> are NOT valid according to the grammar.
        // E052: Empty generic type arguments
        let args = if self.stream.check(&TokenKind::Gt) || self.pending_gt {
            // Emit error for empty generic arguments
            return Err(ParseError::empty_generic_args(self.stream.make_span(start_pos)));
        } else {
            self.comma_separated(|p| {
                // Minimum binding power for expressions inside generic args.
                // Must be > 10 to exclude shift operators `<<` (10) and
                // `>>` (10) which would consume the closing `>>`/`>` as
                // a right-shift operator.  Comparison operators `<`/`>`
                // have BP 6, arithmetic has BP ≤ 13, so BP 11 is the
                // sweet spot: allows +, -, *, /, % but blocks << and >>.
                const GENERIC_ARG_MIN_BP: u8 = 11;

                // Check for HKT placeholder: _
                // This represents a type constructor hole in higher-kinded types
                // Example: Functor<F<_>> where F is a type constructor
                if let Some(Token {
                    kind: TokenKind::Ident(name),
                    ..
                }) = p.stream.peek()
                    && name.as_str() == "_"
                {
                    let span = p.stream.current_span();
                    p.stream.advance();
                    // Create an inferred type to represent the HKT placeholder
                    return Ok(GenericArg::Type(Type::inferred(span)));
                }

                // Check for lifetime argument: 'a, 'b, 'static
                // Lifetimes in type arguments: Type<'a, T>
                if matches!(p.stream.peek_kind(), Some(TokenKind::Lifetime(_))) {
                    let lifetime = p.parse_lifetime()?;
                    return Ok(GenericArg::Lifetime(lifetime));
                }

                // Check if this is definitely a const expression (integer/float literal,
                // or array literal for const generic shapes like [3, 4])
                // Integer, float literals, and array literals should NEVER be parsed as types
                let is_definitely_const = matches!(
                    p.stream.peek_kind(),
                    Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_))
                    | Some(TokenKind::LBracket)
                );

                if is_definitely_const {
                    // Parse as full const expression (handles 0 + n, 2 * k, etc.)
                    // Use bp=7 to stop before `>` and `>=` operators
                    let const_start = p.stream.position();
                    let mut expr = p.parse_expr_bp(GENERIC_ARG_MIN_BP)?;
                    // Comparison heuristic: if `>` follows and has an RHS before another `>`,
                    // treat as comparison (e.g., Proof<3 > 0>)
                    if matches!(p.stream.peek_kind(), Some(TokenKind::Gt) | Some(TokenKind::GtEq) | Some(TokenKind::LtEq)) {
                        let cmp_cp = p.stream.position();
                        let cmp_tok = p.stream.peek_kind().cloned();
                        p.stream.advance();
                        let has_rhs = matches!(p.stream.peek_kind(),
                            Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_))
                            | Some(TokenKind::Ident(_)) | Some(TokenKind::LParen)
                            | Some(TokenKind::True) | Some(TokenKind::False)
                            | Some(TokenKind::Minus) | Some(TokenKind::Bang));
                        if has_rhs {
                            let rhs = p.parse_expr_bp(GENERIC_ARG_MIN_BP)?;
                            if matches!(p.stream.peek_kind(), Some(TokenKind::Gt) | Some(TokenKind::GtGt)) || p.pending_gt {
                                let op = match cmp_tok {
                                    Some(TokenKind::Gt) => verum_ast::BinOp::Gt,
                                    Some(TokenKind::GtEq) => verum_ast::BinOp::Ge,
                                    Some(TokenKind::LtEq) => verum_ast::BinOp::Le,
                                    _ => verum_ast::BinOp::Gt,
                                };
                                let span = p.stream.make_span(const_start);
                                expr = verum_ast::Expr::new(verum_ast::ExprKind::Binary {
                                    op, left: Heap::new(expr), right: Heap::new(rhs),
                                }, span);
                            } else { p.stream.reset_to(cmp_cp); }
                        } else { p.stream.reset_to(cmp_cp); }
                    }
                    Ok(GenericArg::Const(expr))
                } else {
                    // Check for associated type binding: Target=Type
                    // Peek ahead to see if we have Ident = Type pattern
                    let checkpoint = p.stream.position();
                    if let Some(Token {
                        kind: TokenKind::Ident(name),
                        span,
                    }) = p.stream.peek()
                    {
                        let ident_span = *span;
                        let ident_name = name.clone();
                        p.stream.advance(); // consume identifier

                        if p.stream.check(&TokenKind::Eq) {
                            // This is an associated type binding: Name=Type
                            let start_pos = checkpoint;
                            p.stream.advance(); // consume '='

                            let ty = p.parse_type_no_refinement()?;
                            let span = p.stream.make_span(start_pos);
                            let binding =
                                TypeBinding::new(Ident::new(ident_name, ident_span), ty, span);
                            return Ok(GenericArg::Binding(binding));
                        }

                        // Not a binding, restore position and continue
                        p.stream.reset_to(checkpoint);
                    }

                    // Try to parse as type first (with proper backtracking)
                    // But if the parsed type is followed by an arithmetic operator,
                    // treat it as a const expression instead (for dependent types like Eq<n + 0, n>)
                    let checkpoint_before_type = p.stream.position();
                    if let Some(ty) = p.optional(|p2| p2.parse_type()) {
                        // Check if the next token is an arithmetic operator
                        // If so, this should be a const expression, not a type
                        let is_arithmetic_continuation = matches!(
                            p.stream.peek_kind(),
                            Some(TokenKind::Plus) | Some(TokenKind::Minus)
                            | Some(TokenKind::Star) | Some(TokenKind::Slash)
                            | Some(TokenKind::Percent) | Some(TokenKind::EqEq)
                            | Some(TokenKind::BangEq)
                        );
                        if is_arithmetic_continuation {
                            // Reset and parse as const expression
                            p.stream.reset_to(checkpoint_before_type);
                            let arith_start = p.stream.position();
                            let mut expr = p.parse_expr_bp(GENERIC_ARG_MIN_BP)?;
                            // Comparison heuristic for expressions inside generic args
                            if matches!(p.stream.peek_kind(), Some(TokenKind::Gt) | Some(TokenKind::GtEq) | Some(TokenKind::LtEq)) {
                                let cmp_cp = p.stream.position();
                                let cmp_tok = p.stream.peek_kind().cloned();
                                p.stream.advance();
                                let has_rhs = matches!(p.stream.peek_kind(),
                                    Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_))
                                    | Some(TokenKind::Ident(_)) | Some(TokenKind::LParen)
                                    | Some(TokenKind::True) | Some(TokenKind::False)
                                    | Some(TokenKind::Minus) | Some(TokenKind::Bang));
                                if has_rhs {
                                    let rhs = p.parse_expr_bp(GENERIC_ARG_MIN_BP)?;
                                    if matches!(p.stream.peek_kind(), Some(TokenKind::Gt) | Some(TokenKind::GtGt)) || p.pending_gt {
                                        let op = match cmp_tok { Some(TokenKind::Gt) => verum_ast::BinOp::Gt, Some(TokenKind::GtEq) => verum_ast::BinOp::Ge, Some(TokenKind::LtEq) => verum_ast::BinOp::Le, _ => verum_ast::BinOp::Gt };
                                        let span = p.stream.make_span(arith_start);
                                        expr = verum_ast::Expr::new(verum_ast::ExprKind::Binary { op, left: Heap::new(expr), right: Heap::new(rhs) }, span);
                                    } else { p.stream.reset_to(cmp_cp); }
                                } else { p.stream.reset_to(cmp_cp); }
                            }
                            Ok(GenericArg::Const(expr))
                        } else {
                            // Check if `>` after a type is actually a comparison operator
                            // E.g.: Proof<n > 0> -- `n` parsed as type, `>` is comparison
                            if matches!(p.stream.peek_kind(), Some(TokenKind::Gt) | Some(TokenKind::GtEq) | Some(TokenKind::LtEq)) {
                                let cmp_cp = p.stream.position();
                                let cmp_tok = p.stream.peek_kind().cloned();
                                p.stream.advance();
                                let has_rhs = matches!(p.stream.peek_kind(),
                                    Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_))
                                    | Some(TokenKind::Ident(_)) | Some(TokenKind::LParen)
                                    | Some(TokenKind::True) | Some(TokenKind::False)
                                    | Some(TokenKind::Minus) | Some(TokenKind::Bang));
                                if has_rhs {
                                    let rhs = p.parse_expr_bp(GENERIC_ARG_MIN_BP)?;
                                    if matches!(p.stream.peek_kind(), Some(TokenKind::Gt) | Some(TokenKind::GtGt)) || p.pending_gt {
                                        // Convert type to expression and build comparison
                                        let lhs = match &ty.kind {
                                            TypeKind::Path(path) => verum_ast::Expr::new(verum_ast::ExprKind::Path(path.clone()), ty.span),
                                            _ => verum_ast::Expr::new(verum_ast::ExprKind::Path(verum_ast::Path::new(List::from(vec![verum_ast::ty::PathSegment::Name(Ident::new(Text::from("_"), ty.span))]), ty.span)), ty.span),
                                        };
                                        let op = match cmp_tok { Some(TokenKind::Gt) => verum_ast::BinOp::Gt, Some(TokenKind::GtEq) => verum_ast::BinOp::Ge, Some(TokenKind::LtEq) => verum_ast::BinOp::Le, _ => verum_ast::BinOp::Gt };
                                        let span = p.stream.make_span(checkpoint_before_type);
                                        return Ok(GenericArg::Const(verum_ast::Expr::new(verum_ast::ExprKind::Binary { op, left: Heap::new(lhs), right: Heap::new(rhs) }, span)));
                                    }
                                }
                                p.stream.reset_to(cmp_cp);
                            }
                            Ok(GenericArg::Type(ty))
                        }
                    } else {
                        // Check if this looks like a const expression (ident, paren, etc.)
                        // to avoid hanging on invalid tokens
                        let can_be_const = matches!(
                            p.stream.peek_kind(),
                            Some(TokenKind::Ident(_))
                                | Some(TokenKind::LParen)
                                | Some(TokenKind::LBracket)     // Array literal: [3, 4]
                                | Some(TokenKind::Minus)
                                | Some(TokenKind::True)
                                | Some(TokenKind::False)
                                | Some(TokenKind::Integer(_))
                                | Some(TokenKind::Float(_))
                                | Some(TokenKind::Text(_))
                                | Some(TokenKind::Bang)
                                | Some(TokenKind::SelfValue)
                                | Some(TokenKind::Proof)
                                | Some(TokenKind::Exists)
                                | Some(TokenKind::Forall)
                                | Some(TokenKind::Pipe)         // Closure: |n| expr
                                | Some(TokenKind::PipePipe)     // Closure: || expr
                                | Some(TokenKind::Fn)           // Function type: fn(...) -> T
                        );

                        // Quantified expressions (exists, forall) are valid in generic arguments
                        // E.g.: Proof<exists x: Int . x > 10>
                        // Use parse_expr_bp(7) to stop at `>` (comparison operators have BP 6)
                        // This prevents the parser from consuming the closing `>` as part of the expression
                        let is_quantifier_or_paren = matches!(
                            p.stream.peek_kind(),
                            Some(TokenKind::Exists)
                                | Some(TokenKind::Forall)
                                | Some(TokenKind::LParen)
                        );

                        if is_quantifier_or_paren {
                            // Parse full expression, stopping before `>` operators
                            // This handles both direct quantifiers and parenthesized expressions
                            // E.g.: Proof<(exists x: Int . x > 10)>
                            let expr = p.parse_expr_bp(GENERIC_ARG_MIN_BP)?;
                            Ok(GenericArg::Const(expr))
                        } else if can_be_const {
                            // Parse as expression with binding power 7 to stop before `>` and `>=`
                            // This handles:
                            // - Simple identifiers: Vec<N>
                            // - Arithmetic: Vec<N + 1>, List<T, plus(M, N)>
                            // - Function calls: List<T, len(xs)>
                            // - Negation: Array<T, -1>
                            let expr = p.parse_expr_bp(GENERIC_ARG_MIN_BP)?;
                            Ok(GenericArg::Const(expr))
                        } else {
                            Err(ParseError::invalid_syntax(
                                "expected type or const expression in generic argument",
                                p.stream.current_span(),
                            ))
                        }
                    }
                }
            })?
        };

        // E051: Unclosed constraint generic (missing >)
        if self.expect_gt().is_err() {
            return Err(ParseError::unclosed_constraint_generic(self.stream.make_span(start_pos)));
        }

        Ok(args)
    }

    /// Parse a const expression in generic argument position.
    /// This is similar to parse_expr() but stops at generic argument boundaries (comma and >).
    ///
    /// Examples:
    /// - `10` - simple integer literal
    /// - `N + 1` - const expression with identifier
    /// - `SIZE * 2` - const expression with binary operator
    pub(crate) fn parse_refinement_predicate(&mut self) -> ParseResult<RefinementPredicate> {
        let start_pos = self.stream.position();

        // Rule 1: Inline refinement {expr} or {expr1, expr2, ...}
        // Grammar: refinement_predicates = refinement_predicate , { ',' , refinement_predicate } ;
        if self.stream.consume(&TokenKind::LBrace).is_some() {
            // Track brace depth to disambiguate > in nested refinement types like Option<Int{> 0}>
            self.brace_depth += 1;

            // GRAMMAR: refinement_predicates = refinement_predicate , { ',' , refinement_predicate } ;
            // At least one predicate is REQUIRED.
            if self.stream.check(&TokenKind::RBrace) {
                return Err(ParseError::empty_construct(
                    "refinement type",
                    "at least one predicate is required",
                    self.stream.current_span(),
                ));
            }

            // E053: Colon without name `{ : len > 0 }`
            if self.stream.check(&TokenKind::Colon) {
                return Err(ParseError::invalid_refinement_syntax(
                    "unexpected ':' at start of refinement; expected expression",
                    self.stream.current_span(),
                ));
            }

            // Parse a single refinement predicate, handling:
            // - Named predicates: `min: self >= 0` (identifier : expression)
            // - Implicit it comparisons: `> 0` becomes `it > 0`
            // - Regular expressions: `self.len() > 0`
            let parse_single_predicate = |p: &mut Self| -> ParseResult<Expr> {
                // Check for named predicate: `identifier : expression`
                // Grammar: refinement_predicate = identifier , ':' , expression | expression ;
                if let Some(TokenKind::Ident(_)) = p.stream.peek_kind() {
                    if matches!(p.stream.peek_nth(1).map(|t| &t.kind), Some(TokenKind::Colon)) {
                        // Named predicate - skip the name and colon, use just the expression
                        p.stream.advance(); // consume identifier
                        p.stream.advance(); // consume ':'
                        return p.parse_expr();
                    }
                }
                // Try implicit 'it' comparison: {> 0} becomes {it > 0}
                if let Some(implicit_expr) = p.optional(|p| p.parse_implicit_it_comparison()) {
                    return Ok(implicit_expr);
                }
                // Regular expression
                p.parse_expr()
            };

            // Parse first predicate
            let first_expr = parse_single_predicate(self)?;

            // After parsing the first expression, check for:
            // 1. Comma-separated predicates: {> 0, < 100}
            // 2. &&-separated predicates: {>= 0 && <= 100}
            // 3. ||-separated predicates: {< 0 || > 100}
            // 4. Just closing brace: {> 0}

            let mut exprs = vec![first_expr];

            // Parse additional predicates separated by commas
            while self.stream.consume(&TokenKind::Comma).is_some() {
                // E053: Missing value after comma `{ x: 1, }`
                if self.stream.check(&TokenKind::RBrace) {
                    return Err(ParseError::invalid_refinement_syntax(
                        "trailing comma in refinement without predicate",
                        self.stream.current_span(),
                    ));
                }
                let expr = parse_single_predicate(self)?;
                exprs.push(expr);
            }

            // If we have comma-separated predicates, combine them with && and return
            if exprs.len() > 1 {
                self.stream.expect(TokenKind::RBrace)?;
                self.brace_depth -= 1; // Decrement after closing brace
                let span = self.stream.make_span(start_pos);

                use verum_ast::{BinOp, Expr, ExprKind};
                // SAFETY: len() > 1 check above guarantees reduce returns Some
                let combined_expr = exprs
                    .into_iter()
                    .reduce(|acc, expr| {
                        let combined_span = acc.span.merge(expr.span);
                        Expr::new(
                            ExprKind::Binary {
                                op: BinOp::And,
                                left: Box::new(acc),
                                right: Box::new(expr),
                            },
                            combined_span,
                        )
                    })
                    .expect("exprs.len() > 1 guarantees reduce returns Some");

                return Ok(RefinementPredicate::new(combined_expr, span));
            }

            // No commas found, check for && or || operators
            // SAFETY: exprs was initialized with first_expr, and we only reach here if len <= 1
            // Since len >= 1 (from initialization) and len <= 1 (from not entering if branch), len == 1
            let mut result_expr = exprs.into_iter().next()
                .expect("exprs initialized with first_expr guarantees at least one element");

            // Continue parsing && or || operators with implicit it comparisons
            while self.stream.check(&TokenKind::AmpersandAmpersand)
                || self.stream.check(&TokenKind::PipePipe)
            {
                use verum_ast::{BinOp, Expr, ExprKind};

                let op = if self
                    .stream
                    .consume(&TokenKind::AmpersandAmpersand)
                    .is_some()
                {
                    BinOp::And
                } else if self.stream.consume(&TokenKind::PipePipe).is_some() {
                    BinOp::Or
                } else {
                    break;
                };

                // After && or ||, try to parse another implicit it comparison
                let right_expr = if let Some(implicit_expr) =
                    self.optional(|p| p.parse_implicit_it_comparison())
                {
                    implicit_expr
                } else {
                    self.parse_expr()?
                };

                let combined_span = result_expr.span.merge(right_expr.span);
                result_expr = Expr::new(
                    ExprKind::Binary {
                        op,
                        left: Box::new(result_expr),
                        right: Box::new(right_expr),
                    },
                    combined_span,
                );
            }

            self.stream.expect(TokenKind::RBrace)?;
            self.brace_depth -= 1; // Decrement after closing brace
            let span = self.stream.make_span(start_pos);
            return Ok(RefinementPredicate::new(result_expr, span));
        }

        // Rule 2: Lambda-style: where |x| expr
        // Rule 5: Bare where (deprecated): where expr
        if self.stream.consume(&TokenKind::Where).is_some() {
            // Check for lambda: |x|
            // Note: PipePipe (||) is NOT a valid lambda start - empty parameters are
            // meaningless in refinement types since we need a binding variable.
            if self.stream.check(&TokenKind::PipePipe) {
                // Error: || looks like a lambda but has no parameter
                // In refinement types, we need a binding like |x| to refer to the value
                return Err(ParseError::invalid_syntax(
                    "empty lambda parameter in refinement type: expected `|x|` with a binding parameter, not `||`",
                    self.stream.current_span(),
                ));
            }

            if self.stream.check(&TokenKind::Pipe) {
                self.stream.expect(TokenKind::Pipe)?;
                // Allow keywords like `result` as lambda parameter names
                let name = self.consume_ident_or_keyword()?;
                let name_span = self.stream.current_span();
                self.stream.expect(TokenKind::Pipe)?;

                let expr = self.parse_expr()?;
                let span = self.stream.make_span(start_pos);

                return Ok(RefinementPredicate::with_binding(
                    expr,
                    Maybe::Some(Ident::new(name, name_span)),
                    span,
                ));
            } else {
                // Skip optional 'value' keyword in refinement predicates
                // This allows `where value self >= 0.0` as equivalent to `where self >= 0.0`
                if let Some(Token { kind: TokenKind::Ident(name), .. }) = self.stream.peek() {
                    if name.as_str() == "value" {
                        self.stream.advance(); // consume 'value'
                    }
                }

                // Bare where expression (Rule 5 - implicit 'it' binding)
                // Parse the expression, then validate no trailing tokens remain
                //
                // First try implicit 'it' comparison: `where > 0` becomes `where it > 0`
                // This handles: `Int where > 0`, `Int where >= 0 && <= 100`, etc.
                let first_expr = if let Some(implicit_expr) =
                    self.optional(|p| p.parse_implicit_it_where_expression())
                {
                    implicit_expr
                } else {
                    self.parse_expr()?
                };

                // Check for comma-separated predicates: `where self >= 0, self < N`
                // This is common in type definitions like:
                // type ValidIndex<const N: Int> is Int where self >= 0, self < N;
                //
                // IMPORTANT: Only consume commas if the next tokens look like a predicate
                // expression (comparison, identifier followed by comparison, etc.), NOT
                // if they look like a parameter declaration (identifier followed by colon).
                // This prevents `Type where pred, param: Type` from consuming the parameter.
                let mut exprs = vec![first_expr];
                while self.stream.check(&TokenKind::Comma) {
                    // Look ahead: if comma is followed by `ident :` it's a parameter, not another predicate
                    let is_param = matches!(
                        (self.stream.peek_nth_kind(1), self.stream.peek_nth_kind(2)),
                        (Some(&TokenKind::Ident(_)), Some(&TokenKind::Colon))
                    );
                    if is_param {
                        break;
                    }
                    self.stream.advance(); // consume comma
                    // Parse next predicate expression
                    let next_expr = if let Some(implicit_expr) =
                        self.optional(|p| p.parse_implicit_it_where_expression())
                    {
                        implicit_expr
                    } else {
                        self.parse_expr()?
                    };
                    exprs.push(next_expr);
                }

                // Combine all predicates with && if there are multiple
                let expr = if exprs.len() == 1 {
                    // SAFETY: len() == 1 guarantees exactly one element
                    exprs.into_iter().next()
                        .expect("exprs.len() == 1 guarantees one element")
                } else {
                    use verum_ast::{BinOp, Expr, ExprKind};
                    // SAFETY: else branch means len() > 1, so reduce returns Some
                    exprs
                        .into_iter()
                        .reduce(|acc, expr| {
                            let combined_span = acc.span.merge(expr.span);
                            Expr::new(
                                ExprKind::Binary {
                                    op: BinOp::And,
                                    left: Box::new(acc),
                                    right: Box::new(expr),
                                },
                                combined_span,
                            )
                        })
                        .expect("exprs.len() > 1 guarantees reduce returns Some")
                };

                // Validate: after parsing all predicate expressions, we should be at
                // a valid terminator. If there are unexpected tokens (like "x x > 0"
                // where only first "x" is consumed), this is a malformed refinement.
                //
                // Valid terminators for a bare where refinement:
                // - End of input (for parse_type_str)
                // - Semicolon (for type definitions: `type Pos is Int where x > 0;`)
                // - RBrace (for inline refinement end)
                // - RParen (for tuple/function types)
                // - RBracket (for array types)
                // - RAngle (for generics: `List<Int where x > 0>`)
                // - Where (for chained where clauses)
                // - LBrace (for following function/impl body - but NOT directly after bare where)
                //
                // Invalid: Identifier, number, or operator immediately following
                // (indicates malformed syntax like "x x > 0")
                // Note: Comma is no longer valid here since we consumed all comma-separated predicates
                let is_valid_terminator = match self.stream.peek_kind() {
                    None => true,                    // End of input (past last token)
                    Some(TokenKind::Eof) => true,    // Explicit EOF token
                    Some(TokenKind::Semicolon) => true,
                    Some(TokenKind::RBrace) => true,
                    Some(TokenKind::RParen) => true,
                    Some(TokenKind::RBracket) => true,
                    Some(TokenKind::Gt) => true,     // Closing angle bracket for generics
                    Some(TokenKind::Where) => true,
                    Some(TokenKind::LBrace) => true, // Function body following
                    Some(TokenKind::Pipe) => true,   // Pattern alternative
                    _ => false,
                };

                if !is_valid_terminator {
                    return Err(ParseError::invalid_syntax(
                        "unexpected token after refinement predicate: check for malformed lambda syntax (use `|x| expr` not `x expr`)",
                        self.stream.current_span(),
                    ));
                }

                let span = self.stream.make_span(start_pos);
                return Ok(RefinementPredicate::new(expr, span));
            }
        }

        Err(ParseError::invalid_syntax(
            "expected refinement predicate",
            self.stream.current_span(),
        ))
    }

    /// Parse implicit 'it' comparison: {> 0} becomes {it > 0}
    /// Also handles: {!= ""}, {== 42}, etc.
    fn parse_implicit_it_comparison(&mut self) -> ParseResult<verum_ast::Expr> {
        use verum_ast::{BinOp, Expr, ExprKind};

        let start_pos = self.stream.position();

        // Parse comparison operator: >, <, >=, <=, ==, !=
        let (op, op_span) = match self.stream.peek() {
            Some(Token {
                kind: TokenKind::Gt,
                span,
            }) => {
                let span = *span;
                self.stream.advance();
                (BinOp::Gt, span)
            }
            Some(Token {
                kind: TokenKind::Lt,
                span,
            }) => {
                let span = *span;
                self.stream.advance();
                (BinOp::Lt, span)
            }
            Some(Token {
                kind: TokenKind::GtEq,
                span,
            }) => {
                let span = *span;
                self.stream.advance();
                (BinOp::Ge, span)
            }
            Some(Token {
                kind: TokenKind::LtEq,
                span,
            }) => {
                let span = *span;
                self.stream.advance();
                (BinOp::Le, span)
            }
            Some(Token {
                kind: TokenKind::EqEq,
                span,
            }) => {
                let span = *span;
                self.stream.advance();
                (BinOp::Eq, span)
            }
            Some(Token {
                kind: TokenKind::BangEq,
                span,
            }) => {
                let span = *span;
                self.stream.advance();
                (BinOp::Ne, span)
            }
            _ => {
                return Err(ParseError::invalid_syntax(
                    "expected comparison operator",
                    self.stream.current_span(),
                ));
            }
        };

        // Parse right-hand side with binding power 7
        // This allows arithmetic, bitwise ops, etc. but stops before && (bp=5) or || (bp=4)
        // So {>= 0 && <= 100} will parse ">= 0" here, leaving "&& <= 100" for later
        let right = self.parse_expr_bp(7)?;

        // Create implicit 'it' identifier
        let it_ident = Ident::new(Text::from("it"), op_span);
        let it_path = Path::from_ident(it_ident);
        let it_expr = Expr::new(ExprKind::Path(it_path), op_span);

        // Create binary expression: it > 0
        let span = self.stream.make_span(start_pos);
        Ok(Expr::new(
            ExprKind::Binary {
                left: Heap::new(it_expr),
                op,
                right: Heap::new(right),
            },
            span,
        ))
    }

    /// Parse implicit 'it' expression with chained && and || operators.
    /// Handles: `where > 0`, `where >= 0 && <= 100`, `where > 0 || == -1`
    fn parse_implicit_it_where_expression(&mut self) -> ParseResult<verum_ast::Expr> {
        use verum_ast::{BinOp, Expr, ExprKind};

        // First, try to parse an implicit it comparison
        let mut result = self.parse_implicit_it_comparison()?;

        // Continue parsing && or || operators with more implicit it comparisons
        loop {
            if self.stream.consume(&TokenKind::AmpersandAmpersand).is_some() {
                // After &&, try to parse another implicit it comparison
                let right = if let Some(implicit_expr) =
                    self.optional(|p| p.parse_implicit_it_comparison())
                {
                    implicit_expr
                } else {
                    self.parse_expr()?
                };

                let combined_span = result.span.merge(right.span);
                result = Expr::new(
                    ExprKind::Binary {
                        op: BinOp::And,
                        left: Heap::new(result),
                        right: Heap::new(right),
                    },
                    combined_span,
                );
            } else if self.stream.consume(&TokenKind::PipePipe).is_some() {
                // After ||, try to parse another implicit it comparison
                let right = if let Some(implicit_expr) =
                    self.optional(|p| p.parse_implicit_it_comparison())
                {
                    implicit_expr
                } else {
                    self.parse_expr()?
                };

                let combined_span = result.span.merge(right.span);
                result = Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Or,
                        left: Heap::new(result),
                        right: Heap::new(right),
                    },
                    combined_span,
                );
            } else {
                break;
            }
        }

        Ok(result)
    }

    /// Parse a path: foo.bar.baz or just foo
    pub fn parse_path(&mut self) -> ParseResult<Path> {
        let start_pos = self.stream.position();

        let mut segments = Vec::new();

        // Parse first segment
        segments.push(self.parse_path_segment()?);

        // Parse remaining segments: .foo.bar
        // Note: We use . (Dot) for paths in PRIMARY context (e.g., Color.Red)
        // In POSTFIX context, . is still used for field/method access on expressions
        //
        // IMPORTANT: Only consume `.` if followed by an identifier-like token.
        // This prevents `.` from being consumed when it's used as a separator in
        // forall/exists expressions (e.g., `forall i: T . body`), where the `.`
        // separates the quantifier binding from the body expression.
        while self.stream.check(&TokenKind::Dot) && self.is_dot_followed_by_ident() {
            self.stream.advance(); // consume '.'
            segments.push(self.parse_path_segment()?);
        }

        let span = self.stream.make_span(start_pos);
        Ok(Path::new(segments.into_iter().collect::<List<_>>(), span))
    }

    /// Parse a path segment
    pub(crate) fn parse_path_segment(&mut self) -> ParseResult<PathSegment> {
        // Handle both `self` (SelfValue token) and `Self` (SelfType token)
        if self.stream.consume(&TokenKind::SelfValue).is_some()
            || self.stream.consume(&TokenKind::SelfType).is_some()
        {
            return Ok(PathSegment::SelfValue);
        }

        // Handle super keyword
        if self.stream.consume(&TokenKind::Super).is_some() {
            return Ok(PathSegment::Super);
        }

        // Handle crate keyword
        if self.stream.consume(&TokenKind::Cog).is_some() {
            return Ok(PathSegment::Cog);
        }

        // Use consume_ident_or_any_keyword to allow keywords (like 'async') as module names
        let name = self.consume_ident_or_any_keyword()?;
        let span = self.stream.current_span();
        Ok(PathSegment::Name(Ident::new(name, span)))
    }

    /// Parse a single lifetime: 'a, 'static, etc.
    fn parse_lifetime(&mut self) -> ParseResult<Lifetime> {
        if let Some(Token {
            kind: TokenKind::Lifetime(name),
            span,
        }) = self.stream.peek()
        {
            let name = name.clone();
            let span = *span;
            self.stream.advance();
            Ok(Lifetime { name, span })
        } else {
            Err(ParseError::invalid_syntax(
                "expected lifetime (e.g., 'a, 'b, 'static)",
                self.stream.current_span(),
            ))
        }
    }

    /// Parse generic parameters: <T, U, const N: usize>
    pub fn parse_generic_params(&mut self) -> ParseResult<List<GenericParam>> {
        let start_pos = self.stream.position();
        self.stream.expect(TokenKind::Lt)?;

        let params = if self.stream.check(&TokenKind::Gt) || self.pending_gt {
            // E042: Empty generic parameter list <> is not allowed
            let span = self.stream.current_span();
            self.expect_gt()?;
            return Err(ParseError::invalid_syntax(
                "empty generic parameter list '<>' is not allowed; provide at least one type parameter",
                span,
            ));
        } else {
            self.comma_separated(|p| p.parse_generic_param())?
        };

        // E041: Missing generic closing bracket
        self.expect_gt().map_err(|_| {
            ParseError::missing_generic_close(self.stream.current_span())
        })?;

        // E069: Check for duplicate generic parameter names
        {
            let mut seen = std::collections::HashSet::new();
            for param in &params {
                let name_str = match &param.kind {
                    GenericParamKind::Type { name, .. } => Some(name.as_str()),
                    GenericParamKind::HigherKinded { name, .. } => Some(name.as_str()),
                    GenericParamKind::KindAnnotated { name, .. } => Some(name.as_str()),
                    GenericParamKind::Const { name, .. } => Some(name.as_str()),
                    GenericParamKind::Meta { name, .. } => Some(name.as_str()),
                    GenericParamKind::Context { name, .. } => Some(name.as_str()),
                    GenericParamKind::Lifetime { name } => Some(name.name.as_str()),
                    GenericParamKind::Level { name, .. } => Some(name.name.as_str()),
                };
                if let Some(n) = name_str {
                    // Skip duplicate check for `_` — anonymous/placeholder type params
                    // are allowed to appear multiple times (e.g., `type F<_, _>` in HKT)
                    if n != "_" && !seen.insert(n.to_string()) {
                        return Err(ParseError::duplicate_generic_param(n, param.span));
                    }
                }
            }
        }

        Ok(params.into_iter().collect::<List<_>>())
    }

    /// Parse a single generic parameter
    ///
    /// Supports both explicit and implicit parameters:
    /// - Explicit: `T`, `T: Display`, `N: meta Nat`
    /// - Implicit: `{T}`, `{A: Type}`, `{n: meta Nat}`
    ///
    /// Implicit params use braces: `{T}`, `{A: Type}`, `{n: meta Nat}` — inferred from usage.
    /// Explicit params are bare: `T`, `T: Display`, `N: meta Nat`.
    /// Dependent type params use `meta` keyword to indicate compile-time-only values.
    fn parse_generic_param(&mut self) -> ParseResult<GenericParam> {
        let start_pos = self.stream.position();

        // Check for implicit parameter: {T}, {T: Type}, {n: meta Nat}
        // Implicit parameters are inferred from usage context
        let is_implicit = self.stream.consume(&TokenKind::LBrace).is_some();

        // Check for lifetime parameter: 'a, 'static
        if matches!(self.stream.peek_kind(), Some(TokenKind::Lifetime(_))) {
            let lifetime = self.parse_lifetime()?;
            if is_implicit {
                self.stream.expect(TokenKind::RBrace)?;
            }
            let span = self.stream.make_span(start_pos);
            return Ok(GenericParam {
                kind: GenericParamKind::Lifetime { name: lifetime },
                is_implicit,
                span,
            });
        }

        // Check for context parameter: using C
        // Context polymorphism: generic params can be context-typed for abstracting over context sets
        // Enables higher-order functions to propagate contexts from callbacks
        // Example: fn map<T, U, using C>(iter: I, f: fn(T) -> U using C) -> MapIter using C
        if self.stream.consume(&TokenKind::Using).is_some() {
            let name = self.consume_ident()?;
            let name_span = self.stream.current_span();
            if is_implicit {
                self.stream.expect(TokenKind::RBrace)?;
            }
            let span = self.stream.make_span(start_pos);
            return Ok(GenericParam {
                kind: GenericParamKind::Context {
                    name: Ident::new(name, name_span),
                },
                is_implicit,
                span,
            });
        }

        // Check for universe level parameter: universe u
        // Universe polymorphism -- preferred form per verum-ext.md §2.1.
        // Grammar: universe_param = 'universe' , identifier ;
        // Example: @universe_poly fn id<universe u, A: Type(u)>(x: A) -> A { x }
        // Note: 'universe' is context-sensitive -- only acts as a keyword inside generic params.
        if let Some(Token {
            kind: TokenKind::Ident(kw),
            ..
        }) = self.stream.peek()
            && kw.as_str() == "universe"
        {
            // Consume the contextual 'universe' keyword
            self.stream.advance();
            let name = self.consume_ident()?;
            let name_span = self.stream.current_span();
            if is_implicit {
                self.stream.expect(TokenKind::RBrace)?;
            }
            let span = self.stream.make_span(start_pos);
            return Ok(GenericParam {
                kind: GenericParamKind::Level {
                    name: Ident::new(name, name_span),
                },
                is_implicit,
                span,
            });
        }

        // Check for placeholder type parameter: _
        // This is used for HKT: type F<_>
        if let Some(Token {
            kind: TokenKind::Ident(name),
            ..
        }) = self.stream.peek()
            && name.as_str() == "_"
        {
            let span = self.stream.current_span();
            self.stream.advance();
            if is_implicit {
                self.stream.expect(TokenKind::RBrace)?;
            }
            return Ok(GenericParam {
                kind: GenericParamKind::Type {
                    name: Ident::new(Text::from("_"), span),
                    bounds: List::new(),
                    default: Maybe::None,
                },
                is_implicit,
                span: self.stream.make_span(start_pos),
            });
        }

        // Try: N: meta Type or N: meta Type{refinement}
        // Also handles HKT syntax: F<_> where F is a type constructor
        if self.is_ident() {
            let checkpoint = self.stream.position();
            let name = self.consume_ident()?;
            let name_span = self.stream.current_span();

            // Check for HKT syntax: F<_> or F<_, _>
            // This declares a type parameter that is itself a type constructor
            if self.stream.check(&TokenKind::Lt) {
                // Parse the generic args to get the HKT structure
                // Example: F<_> -> name="F", args=[Inferred]
                let args = self.parse_generic_args()?;

                // Validate that all arguments are placeholders (_)
                // HKT parameters must use _ to indicate type constructor arity
                // T<U> is invalid - should be T<_> for HKT or just T for a regular type parameter
                for arg in args.iter() {
                    if let GenericArg::Type(ty) = arg {
                        // Check if this is an inferred type (placeholder _)
                        if !matches!(ty.kind, TypeKind::Inferred) {
                            return Err(ParseError::invalid_syntax(
                                "invalid generic parameter syntax: type parameters cannot have non-placeholder generic arguments. Use _ for higher-kinded types (e.g., F<_>)",
                                ty.span,
                            ));
                        }
                    }
                }

                // The name with its type constructor arity is represented
                // by storing just the name - the type checker will infer
                // that this is a type constructor based on usage
                // For now, we just record the name and continue
                // The presence of generic args indicates this is an HKT parameter

                // Check for bounds after the HKT declaration
                let bounds = if self.stream.consume(&TokenKind::Colon).is_some() {
                    // E089: Leading plus after colon
                    if self.stream.check(&TokenKind::Plus) {
                        return Err(ParseError::leading_pipe_no_context(self.stream.current_span()));
                    }
                    // E090: Check for missing constraint after colon
                    if matches!(
                        self.stream.peek_kind(),
                        Some(TokenKind::Gt)
                            | Some(TokenKind::Comma)
                            | Some(TokenKind::RBrace)
                            | None
                    ) {
                        return Err(ParseError::missing_constraint(self.stream.make_span(start_pos)));
                    }
                    if self.stream.check(&TokenKind::Bang)
                        || self.is_ident()
                        || self.stream.check(&TokenKind::Fn)
                    {
                        self.parse_type_bounds_or_type()?
                    } else {
                        List::new()
                    }
                } else {
                    List::new()
                };

                // CRITICAL FIX: Return HigherKinded, not Type!
                // F<_> should create a HigherKinded param with arity 1
                // F<_, _> should create a HigherKinded param with arity 2
                // Higher-kinded types: `F<_>` creates a HigherKinded param with arity 1
                let arity = args.len();
                if is_implicit {
                    self.stream.expect(TokenKind::RBrace)?;
                }
                let span = self.stream.make_span(start_pos);
                return Ok(GenericParam {
                    kind: GenericParamKind::HigherKinded {
                        name: Ident::new(name, name_span),
                        arity,
                        bounds,
                    },
                    is_implicit,
                    span,
                });
            }

            // E092: Check for double colon before consuming
            if self.stream.check(&TokenKind::ColonColon) {
                return Err(ParseError::double_colon_constraint(self.stream.current_span()));
            }

            if self.stream.consume(&TokenKind::Colon).is_some() {
                // E092: Check for double colon after first colon
                if self.stream.check(&TokenKind::Colon) {
                    return Err(ParseError::double_colon_constraint(self.stream.current_span()));
                }

                // E089: Leading plus after colon (like `T: + Clone`)
                if self.stream.check(&TokenKind::Plus) {
                    return Err(ParseError::leading_pipe_no_context(self.stream.current_span()));
                }

                // E090: Check for missing constraint after colon
                if matches!(
                    self.stream.peek_kind(),
                    Some(TokenKind::Gt)
                        | Some(TokenKind::Comma)
                        | Some(TokenKind::RBrace)
                        | None
                ) {
                    return Err(ParseError::missing_constraint(self.stream.make_span(start_pos)));
                }

                // Check for universe level parameter: u: Level
                // Grammar: level_param = identifier , ':' , 'Level' ;
                // Enables universe polymorphism in type/function declarations.
                if let Some(Token {
                    kind: TokenKind::Ident(bound_name),
                    ..
                }) = self.stream.peek()
                {
                    if bound_name.as_str() == "Level" {
                        // Consume 'Level'
                        self.stream.advance();
                        if is_implicit {
                            self.stream.expect(TokenKind::RBrace)?;
                        }
                        let span = self.stream.make_span(start_pos);
                        return Ok(GenericParam {
                            kind: GenericParamKind::Level {
                                name: Ident::new(name, name_span),
                            },
                            is_implicit,
                            span,
                        });
                    }
                }

                // Check for kind annotation: F: Type -> Type
                // Grammar: kind_annotated_param = identifier ':' kind_expr
                //          kind_expr = 'Type' [ '->' kind_expr ] | '(' kind_expr ')'
                // Disambiguates from protocol bounds (T: Display) by the presence of
                // the `Type` keyword (TokenKind::Type) as the first token after ':'.
                if self.stream.check(&TokenKind::Type) {
                    let kind_ann = self.parse_kind_annotation()?;

                    // Optional additional protocol bounds after '+': F: Type -> Type + Functor
                    let bounds = if self.stream.consume(&TokenKind::Plus).is_some() {
                        self.parse_type_bounds_or_type()?
                    } else {
                        List::new()
                    };

                    if is_implicit {
                        self.stream.expect(TokenKind::RBrace)?;
                    }
                    let span = self.stream.make_span(start_pos);
                    return Ok(GenericParam {
                        kind: GenericParamKind::KindAnnotated {
                            name: Ident::new(name, name_span),
                            kind: kind_ann,
                            bounds,
                        },
                        is_implicit,
                        span,
                    });
                }

                // Check for meta parameter
                if self.stream.consume(&TokenKind::Meta).is_some() {
                    let ty = self.parse_type()?;

                    // Optional inline refinement: {> 0}
                    // Note: for implicit params, the outer braces are already consumed
                    // so we need to check for inner refinement braces separately
                    let refinement = if !is_implicit && self.stream.check(&TokenKind::LBrace) {
                        self.stream.expect(TokenKind::LBrace)?;
                        let expr = self.parse_expr()?;
                        self.stream.expect(TokenKind::RBrace)?;
                        Some(Box::new(expr))
                    } else {
                        None
                    };

                    if is_implicit {
                        self.stream.expect(TokenKind::RBrace)?;
                    }
                    let span = self.stream.make_span(start_pos);
                    return Ok(GenericParam {
                        kind: GenericParamKind::Meta {
                            name: Ident::new(name, name_span),
                            ty,
                            refinement,
                        },
                        is_implicit,
                        span,
                    });
                }

                // Type parameter with bounds: T: Display or T: Display + Debug
                // Also supports function type bounds: F: Fn(T) -> U
                let bounds = if self.stream.check(&TokenKind::Bang)
                    || self.is_ident()
                    || self.stream.check(&TokenKind::Fn)
                {
                    self.parse_type_bounds_or_type()?
                } else {
                    List::new()
                };

                // Optional default: T = Default
                let default = if self.stream.consume(&TokenKind::Eq).is_some() {
                    Some(self.parse_type()?)
                } else {
                    None
                };

                if is_implicit {
                    self.stream.expect(TokenKind::RBrace)?;
                }
                let span = self.stream.make_span(start_pos);
                return Ok(GenericParam {
                    kind: GenericParamKind::Type {
                        name: Ident::new(name, name_span),
                        bounds,
                        default,
                    },
                    is_implicit,
                    span,
                });
            } else {
                // Just a type parameter without bounds: T
                if is_implicit {
                    self.stream.expect(TokenKind::RBrace)?;
                }
                let span = self.stream.make_span(start_pos);
                return Ok(GenericParam {
                    kind: GenericParamKind::Type {
                        name: Ident::new(name, name_span),
                        bounds: List::new(),
                        default: Maybe::None,
                    },
                    is_implicit,
                    span,
                });
            }
        }

        // const N: Type (deprecated - use meta instead)
        if self.stream.consume(&TokenKind::Const).is_some() {
            let name = self.consume_ident()?;
            let name_span = self.stream.current_span();

            self.stream.expect(TokenKind::Colon)?;
            let ty = self.parse_type()?;

            if is_implicit {
                self.stream.expect(TokenKind::RBrace)?;
            }
            let span = self.stream.make_span(start_pos);
            return Ok(GenericParam {
                kind: GenericParamKind::Const {
                    name: Ident::new(name, name_span),
                    ty,
                },
                is_implicit,
                span,
            });
        }

        // If we started with { but didn't find a valid param, that's an error
        if is_implicit {
            return Err(ParseError::invalid_syntax(
                "expected identifier after '{' for implicit generic parameter",
                self.stream.current_span(),
            ));
        }

        Err(ParseError::invalid_syntax(
            "expected generic parameter",
            self.stream.current_span(),
        ))
    }

    /// Parse type bounds: Protocol1 + Protocol2 + !Protocol3
    pub fn parse_type_bounds(&mut self) -> ParseResult<List<TypeBound>> {
        let start_pos = self.stream.position();
        let mut bounds = Vec::new();

        // E089: Leading plus is not valid
        if self.stream.check(&TokenKind::Plus) {
            return Err(ParseError::leading_pipe_no_context(self.stream.current_span()));
        }

        loop {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }

            let bound_start = self.stream.position();
            let pos_before = self.stream.position();

            // Negative bound: !Protocol
            if self.stream.consume(&TokenKind::Bang).is_some() {
                // E093: Check for double negation
                if self.stream.check(&TokenKind::Bang) {
                    return Err(ParseError::double_negation_bound(self.stream.current_span()));
                }
                // E090: Missing bound after `!`
                if !self.is_ident() {
                    return Err(ParseError::missing_constraint(self.stream.make_span(bound_start)));
                }
                let path = self.parse_path()?;
                let span = self.stream.make_span(bound_start);

                bounds.push(TypeBound {
                    kind: TypeBoundKind::NegativeProtocol(path),
                    span,
                });
            } else {
                // Positive bound: Protocol or Protocol<T>
                // Use parse_type_no_refinement to correctly handle generic arguments
                let bound_type = self.parse_type_no_refinement()?;
                let bound = self.type_to_type_bound(bound_type)?;
                bounds.push(bound);
            }

            // Safety: Ensure we made forward progress
            if self.stream.position() == pos_before {
                return Err(ParseError::invalid_syntax(
                    "parser made no progress in type bounds",
                    self.stream.current_span(),
                ));
            }

            // Check for more bounds: + Protocol
            if self.stream.consume(&TokenKind::Plus).is_none() {
                break;
            }

            // GRAMMAR: bounds = bound , { '+' , bound } ;
            // After consuming '+', another bound is expected.
            // Allow trailing '+' (common pattern) - just break the loop
            let is_lifetime = matches!(self.stream.peek_kind(), Some(TokenKind::Lifetime(_)));
            if !self.stream.check(&TokenKind::Bang)
                && !self.is_ident()
                && !self.stream.check(&TokenKind::SelfType)
                && !is_lifetime
            {
                // Trailing '+' is invalid per grammar: bounds = bound , { '+' , bound } ;
                return Err(ParseError::invalid_syntax(
                    "trailing `+` in type bounds is not allowed; expected another bound after `+`",
                    self.stream.current_span(),
                ));
            }
            // Handle lifetime bounds: consume and skip (lifetimes are informational in Verum)
            if is_lifetime {
                self.stream.advance(); // consume the lifetime token
                continue;
            }
        }

        if bounds.is_empty() {
            return Err(ParseError::invalid_syntax(
                "expected at least one type bound",
                self.stream.current_span(),
            ));
        }

        Ok(bounds.into_iter().collect::<List<_>>())
    }

    /// Parse type bounds or a single type (for function type bounds like `F: Fn(T) -> U`).
    /// This method handles:
    /// - Protocol bounds: `T: Clone + Debug`
    /// - Function type bounds: `F: Fn(T) -> U`
    /// - Negative bounds: `T: !Send`
    /// - Other complex type bounds
    ///
    /// If the bound is a simple path (protocol name), it creates a Protocol bound.
    /// If it's a complex type (like a function type), it creates an Equality bound.
    pub fn parse_type_bounds_or_type(&mut self) -> ParseResult<List<TypeBound>> {
        let start_pos = self.stream.position();

        // E089: Leading plus is not valid
        if self.stream.check(&TokenKind::Plus) {
            return Err(ParseError::leading_pipe_no_context(self.stream.current_span()));
        }

        // Check for negative bound: !Protocol
        // This must come before parse_type_no_refinement because ! can be a unary operator in types
        let mut bounds = if self.stream.consume(&TokenKind::Bang).is_some() {
            // E093: Check for double negation
            if self.stream.check(&TokenKind::Bang) {
                return Err(ParseError::double_negation_bound(self.stream.current_span()));
            }
            // E090: Missing bound after `!`
            if !self.is_ident() {
                return Err(ParseError::missing_constraint(self.stream.make_span(start_pos)));
            }
            let path = self.parse_path()?;
            let span = self.stream.make_span(start_pos);
            vec![TypeBound {
                kind: TypeBoundKind::NegativeProtocol(path),
                span,
            }]
        } else {
            // Try to parse as a type first
            // This handles both simple paths (Protocol) and complex types (fn(T) -> U)
            // NOTE: Use parse_type_no_refinement to avoid consuming { as a refinement in impl blocks
            let first_type = self.parse_type_no_refinement()?;
            vec![self.type_to_type_bound(first_type)?]
        };

        // Check if this is followed by a + (multiple protocol bounds)
        if self.stream.check(&TokenKind::Plus) {
            // Multiple protocol bounds
            while self.stream.consume(&TokenKind::Plus).is_some() {
                let bound_start = self.stream.position();

                // Negative bound: !Protocol
                if self.stream.consume(&TokenKind::Bang).is_some() {
                    // E093: Check for double negation
                    if self.stream.check(&TokenKind::Bang) {
                        return Err(ParseError::double_negation_bound(self.stream.current_span()));
                    }
                    // E090: Missing bound after `!`
                    if !self.is_ident() {
                        return Err(ParseError::missing_constraint(self.stream.make_span(bound_start)));
                    }
                    let path = self.parse_path()?;
                    let span = self.stream.make_span(bound_start);
                    bounds.push(TypeBound {
                        kind: TypeBoundKind::NegativeProtocol(path),
                        span,
                    });
                } else if matches!(self.stream.peek_kind(), Some(TokenKind::Lifetime(_))) {
                    // Lifetime bound: 'static, 'a, etc. - consume and skip (informational in Verum)
                    self.stream.advance();
                } else if self.is_ident() || self.stream.check(&TokenKind::SelfType) {
                    // Positive bound: Protocol or Protocol<T>
                    // Use parse_type_no_refinement to correctly handle generic arguments
                    let bound_type = self.parse_type_no_refinement()?;
                    let bound = self.type_to_type_bound(bound_type)?;
                    bounds.push(bound);
                } else {
                    // GRAMMAR: bounds = bound , { '+' , bound } ;
                    // After '+', another bound is required. Trailing '+' is invalid.
                    return Err(ParseError::invalid_syntax(
                        "trailing `+` in type bounds is not allowed; expected another bound after `+`",
                        self.stream.current_span(),
                    ));
                }
            }
        }

        Ok(bounds.into_iter().collect::<List<_>>())
    }

    /// Convert a Type to a TypeBound.
    /// - If it's a simple path, creates a Protocol bound
    /// - If it's a generic type (like Iterator<Item = T>), creates a GenericProtocol bound
    /// - Otherwise, creates an Equality bound (for complex types like function types)
    fn type_to_type_bound(&self, ty: Type) -> ParseResult<TypeBound> {
        let span = ty.span;

        match &ty.kind {
            // If it's a simple path, convert to Protocol bound
            TypeKind::Path(path) => Ok(TypeBound {
                kind: TypeBoundKind::Protocol(path.clone()),
                span,
            }),
            // If it's a generic type (like Iterator<Item = Self.Item>), use GenericProtocol
            TypeKind::Generic { base, .. } => {
                // Only use GenericProtocol if the base is a path (protocol name)
                if matches!(&base.kind, TypeKind::Path(_)) {
                    Ok(TypeBound {
                        kind: TypeBoundKind::GenericProtocol(ty),
                        span,
                    })
                } else {
                    // Complex base type - use Equality bound
                    Ok(TypeBound {
                        kind: TypeBoundKind::Equality(ty),
                        span,
                    })
                }
            }
            // For complex types (function types, etc.), use Equality bound
            _ => Ok(TypeBound {
                kind: TypeBoundKind::Equality(ty),
                span,
            }),
        }
    }

    /// Parse a capability list for capability-restricted types.
    ///
    /// Parses: `[Read, Write, Admin]` or `[Read | Write, Admin]`
    ///
    /// Grammar:
    /// ```ebnf
    /// capability_list = '[' , capability_item , { ',' , capability_item } , ']' ;
    /// capability_item = capability_name | capability_or_expr ;
    /// capability_name = 'Read' | 'Write' | ... | identifier ;
    /// capability_or_expr = capability_name , '|' , capability_name , { '|' , capability_name } ;
    /// ```
    ///
    /// Capability attenuation restricts a context to a subset of its operations.
    /// E.g., `Database with [Read]` restricts to read-only access. Sub-contexts (like
    /// `FS.ReadOnly`) are resolved at compile-time with zero runtime overhead.
    fn parse_capability_list(&mut self) -> ParseResult<verum_ast::expr::CapabilitySet> {
        use verum_ast::expr::{Capability, CapabilitySet};

        let start_pos = self.stream.position();

        // Expect '['
        self.stream.expect(TokenKind::LBracket)?;

        // E076: Empty capability list
        if self.stream.check(&TokenKind::RBracket) {
            return Err(ParseError::empty_capability(self.stream.make_span(start_pos)));
        }

        let mut capabilities = Vec::new();

        // Parse first capability
        capabilities.push(self.parse_capability_name()?);

        // Parse additional capabilities separated by ','
        // Note: '|' within a capability is handled in parse_capability_name
        while self.stream.consume(&TokenKind::Comma).is_some() {
            if !self.tick() || self.is_aborted() {
                break;
            }
            // E053: Double comma in capability list
            if self.stream.check(&TokenKind::Comma) {
                return Err(ParseError::double_comma_capability(self.stream.make_span(start_pos)));
            }
            // E054: Trailing comma in capability list
            if self.stream.check(&TokenKind::RBracket) {
                return Err(ParseError::trailing_comma_capability(self.stream.make_span(start_pos)));
            }
            capabilities.push(self.parse_capability_name()?);
        }

        // E075: Unclosed capability list
        if self.stream.consume(&TokenKind::RBracket).is_none() {
            return Err(ParseError::unclosed_capability(self.stream.make_span(start_pos)));
        }

        let span = self.stream.make_span(start_pos);
        Ok(CapabilitySet::new(capabilities.into_iter().collect(), span))
    }

    /// Parse a single capability name.
    ///
    /// Handles both standard capabilities (Read, Write, etc.) and custom capabilities.
    /// Standard capabilities are recognized by name and converted to the appropriate enum variant.
    ///
    /// Grammar:
    /// ```ebnf
    /// capability_name = 'Read' | 'Write' | 'ReadWrite' | 'Admin' | 'Transaction'
    ///                 | 'Network' | 'FileSystem' | 'Query' | 'Execute'
    ///                 | 'Logging' | 'Metrics' | 'Config' | 'Cache' | 'Auth'
    ///                 | identifier ;
    /// ```
    fn parse_capability_name(&mut self) -> ParseResult<verum_ast::expr::Capability> {
        use verum_ast::expr::Capability;

        let name = self.consume_ident()?;
        let span = self.stream.current_span();

        let capability = match name.as_str() {
            "Read" | "ReadOnly" => Capability::ReadOnly,
            "Write" | "WriteOnly" => Capability::WriteOnly,
            "ReadWrite" => Capability::ReadWrite,
            "Admin" => Capability::Admin,
            "Transaction" => Capability::Transaction,
            "Network" => Capability::Network,
            "FileSystem" => Capability::FileSystem,
            "Query" => Capability::Query,
            "Execute" => Capability::Execute,
            "Logging" => Capability::Logging,
            "Metrics" => Capability::Metrics,
            "Config" => Capability::Config,
            "Cache" => Capability::Cache,
            "Auth" => Capability::Auth,
            _ => Capability::Custom(name),
        };

        Ok(capability)
    }

    /// Parse associated type bindings for dyn protocol types.
    ///
    /// Parses comma-separated type bindings: Item = Int, State = String
    /// Used in: dyn Container<Item = Int> + Display
    ///
    /// Associated type bindings for dyn protocol types: `dyn Container<Item = Int> + Display`
    /// Grammar: type_bindings = type_binding , { ',' , type_binding } ;
    /// type_binding = identifier , '=' , type_expr ;
    fn parse_type_bindings(&mut self) -> ParseResult<List<TypeBinding>> {
        let bindings = self.comma_separated(|p| p.parse_type_binding())?;
        Ok(bindings.into_iter().collect::<List<_>>())
    }

    /// Parse a single type binding: Item = Int
    ///
    /// Single type binding: `Item = Int` — associates a concrete type with an associated type name
    fn parse_type_binding(&mut self) -> ParseResult<TypeBinding> {
        let start_pos = self.stream.position();

        // Parse the associated type name
        let name_text = self.consume_ident()?;
        let name_span = self.stream.current_span();
        let name = Ident::new(name_text, name_span);

        // Expect '='
        self.stream.expect(TokenKind::Eq)?;

        // Parse the bound type
        let ty = self.parse_type()?;

        let span = self.stream.make_span(start_pos);

        Ok(TypeBinding { name, ty, span })
    }

    /// Parse a where clause: where T: Protocol, U: Bound
    pub fn parse_where_clause(&mut self) -> ParseResult<WhereClause> {
        let start_pos = self.stream.position();

        self.stream.expect(TokenKind::Where)?;

        // E038: Empty where clause - `where {}`
        if self.stream.check(&TokenKind::LBrace) {
            return Err(ParseError::invalid_where_clause(
                "empty where clause",
                self.stream.current_span(),
            ));
        }

        let predicates = self.comma_separated(|p| p.parse_where_predicate())?;

        let span = self.stream.make_span(start_pos);
        Ok(WhereClause {
            predicates: predicates.into_iter().collect::<List<_>>(),
            span,
        })
    }

    /// Parse a where predicate with v6.0-BALANCED disambiguation.
    ///
    /// Where predicates disambiguate via keyword prefix:
    /// - `where type T: Protocol` — generic type constraint (type bound)
    /// - `where type A.Item = B` — associated type equality
    /// - `where value_expr` — value-level constraint (refinement predicate)
    ///   The `type` prefix distinguishes type bounds from value predicates.
    fn parse_where_predicate(&mut self) -> ParseResult<WherePredicate> {
        let start_pos = self.stream.position();

        // 1. Generic type constraint: where type T: Protocol
        //    or associated type equality: where type A.Item = B
        if self.stream.consume(&TokenKind::Type).is_some() {
            // Use parse_type_no_sigma to prevent `T: Clone` from being parsed as a sigma type.
            // In `where type T: Clone`, `T` is a type identifier and `: Clone` is a type bound,
            // not a sigma type pattern.
            let ty = self.parse_type_no_sigma()?;

            // Check for associated type equality: type A.Item = B
            if self.stream.check(&TokenKind::Eq) {
                self.stream.advance(); // consume '='
                let concrete_ty = self.parse_type_no_refinement()?;

                let span = self.stream.make_span(start_pos);
                return Ok(WherePredicate {
                    kind: WherePredicateKind::Type {
                        ty: ty.clone(),
                        bounds: vec![TypeBound {
                            kind: TypeBoundKind::Equality(concrete_ty),
                            span,
                        }]
                        .into_iter()
                        .collect::<List<_>>(),
                    },
                    span,
                });
            }

            // Regular type constraint: type T: Protocol
            self.stream.expect(TokenKind::Colon)?;

            // E038: Missing constraint after colon - `where type T: {}`
            if self.stream.check(&TokenKind::LBrace) || self.stream.check(&TokenKind::Where) {
                return Err(ParseError::invalid_where_clause(
                    "missing constraint after ':'",
                    self.stream.current_span(),
                ));
            }

            // E038: Invalid bound - `where type T: 123`
            if let Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_)) | Some(TokenKind::Text(_)) =
                self.stream.peek_kind()
            {
                return Err(ParseError::invalid_where_clause(
                    "expected type constraint, found literal",
                    self.stream.current_span(),
                ));
            }

            let bounds = self.parse_type_bounds_or_type()?;

            let span = self.stream.make_span(start_pos);
            return Ok(WherePredicate {
                kind: WherePredicateKind::Type { ty, bounds },
                span,
            });
        }

        // 2. Meta-parameter refinement: where meta N > 0
        if self.stream.consume(&TokenKind::Meta).is_some() {
            // Use parse_expr_for_meta_predicate to avoid consuming `is` as pattern test operator.
            // This allows `type X where meta N > 0 is { ... }` to parse correctly.
            let constraint = self.parse_expr_for_meta_predicate()?;

            let span = self.stream.make_span(start_pos);
            return Ok(WherePredicate {
                kind: WherePredicateKind::Meta { constraint },
                span,
            });
        }

        // 3. Postcondition: where ensures result >= 0
        if self.stream.consume(&TokenKind::Ensures).is_some() {
            // Use parse_expr_no_struct to avoid consuming function body as struct literal
            let postcondition = self.parse_expr_no_struct()?;

            let span = self.stream.make_span(start_pos);
            return Ok(WherePredicate {
                kind: WherePredicateKind::Ensures { postcondition },
                span,
            });
        }

        // 4. Explicit value refinement: where value it > 0
        // Note: "value" is not a keyword, so we check for it as an identifier
        if self.is_ident()
            && let Some(Token {
                kind: TokenKind::Ident(name),
                ..
            }) = self.stream.peek()
        {
            if name.as_str() == "value" {
                self.stream.advance();
                // Use parse_expr_no_struct to avoid consuming function body as struct literal
                let predicate = self.parse_expr_no_struct()?;

                let span = self.stream.make_span(start_pos);
                return Ok(WherePredicate {
                    kind: WherePredicateKind::Value { predicate },
                    span,
                });
            }
        }

        // 6. Implicit type constraint: where T: Protocol (without "type" keyword)
        //    Also handles associated type equality: where T.Item = Int
        // Try to parse as type constraint if we have an identifier followed by colon or dot
        if self.is_ident() || self.stream.check(&TokenKind::SelfType) {
            let checkpoint = self.stream.position();
            // Use parse_type_no_sigma to prevent `T: Eq` from being parsed as a sigma type.
            // In `where T: Eq`, `T` is a type identifier and `: Eq` is a type bound,
            // not a sigma type pattern.
            if let Ok(ty) = self.parse_type_no_sigma() {
                // Check for associated type equality: T.Item = Type
                // Note: parse_type() will have already consumed T.Item as a Qualified type if present
                // So we check if the parsed type is a Qualified type followed by '='
                if let TypeKind::Qualified {
                    self_ty,
                    trait_ref,
                    assoc_name,
                } = &ty.kind
                {
                    // Check if this is followed by '=' (associated type equality)
                    if self.stream.check(&TokenKind::Eq) {
                        self.stream.advance(); // consume '='

                        // Parse the concrete type without refinement to avoid consuming
                        // the function/impl body brace as a refinement predicate
                        let concrete_ty = self.parse_type_no_refinement()?;

                        // Create an equality bound: T.Item = Type
                        let span = self.stream.make_span(start_pos);
                        return Ok(WherePredicate {
                            kind: WherePredicateKind::Type {
                                ty: ty.clone(),
                                bounds: vec![TypeBound {
                                    kind: TypeBoundKind::Equality(concrete_ty),
                                    span,
                                }]
                                .into_iter()
                                .collect::<List<_>>(),
                            },
                            span,
                        });
                    }
                }

                // Regular type constraint: T: Protocol
                if self.stream.check(&TokenKind::Colon) {
                    self.stream.advance();
                    let bounds = self.parse_type_bounds_or_type()?;

                    let span = self.stream.make_span(start_pos);
                    return Ok(WherePredicate {
                        kind: WherePredicateKind::Type { ty, bounds },
                        span,
                    });
                }
            }
            // Restore if we couldn't parse type constraint
            self.stream.reset_to(checkpoint);
        }

        // 7. Implicit value predicate: where x > 0 (without "value" keyword)
        // This is the fallback case for value refinement expressions
        // Use parse_expr_for_meta_predicate to avoid consuming `is` keyword as pattern test
        // operator. In where clause context, `is` is the type definition keyword that follows
        // the where clause (e.g., `where C > 0 is { ... }`), not a pattern test.
        let predicate = self.parse_expr_for_meta_predicate()?;
        let span = self.stream.make_span(start_pos);
        Ok(WherePredicate {
            kind: WherePredicateKind::Value { predicate },
            span,
        })
    }

    /// Parse context list: [Database, Logger, Auth]
    ///
    /// Verum uses a Context System for dependency injection, NOT algebraic effects.
    /// Contexts are declared with `using [...]` and provided with `provide`.
    /// Verum uses a Context System for dependency injection (NOT algebraic effects).
    /// Single context: `using Ctx` (brackets optional). Multiple: `using [Ctx1, Ctx2]`.
    fn parse_context_list(&mut self) -> ParseResult<List<Path>> {
        self.stream.expect(TokenKind::LBracket)?;

        let contexts = if self.stream.check(&TokenKind::RBracket) {
            Vec::new()
        } else {
            self.comma_separated(|p| p.parse_path())?
        };

        self.stream.expect(TokenKind::RBracket)?;

        if contexts.is_empty() {
            return Err(ParseError::invalid_syntax(
                "context list cannot be empty - use 'using [Context1, Context2]'",
                self.stream.current_span(),
            ));
        }

        Ok(contexts.into_iter().collect::<List<_>>())
    }

    /// Parse protocol type: impl Display + Debug or dyn Display + Debug
    fn parse_protocol_type(&mut self) -> ParseResult<Type> {
        let start_pos = self.stream.position();

        // Static protocol: impl Display + Debug
        // Note: Check both TokenKind::Implement and contextual keyword "impl"
        let is_impl = self.stream.consume(&TokenKind::Implement).is_some() || {
            if self.is_ident() {
                if let Some(Token {
                    kind: TokenKind::Ident(name),
                    ..
                }) = self.stream.peek()
                {
                    if name.as_str() == "impl" {
                        self.stream.advance();
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        };

        if is_impl {
            let bounds = self.parse_type_bounds()?;
            let span = self.stream.make_span(start_pos);

            return Ok(Type::new(
                TypeKind::Bounded {
                    base: Box::new(Type::inferred(span)),
                    bounds,
                },
                span,
            ));
        }

        // Dynamic protocol: dyn Display + Debug
        // With optional associated type bindings: dyn Container<Item = Int> + Display
        // Note: For dyn types, we parse protocol paths WITHOUT generic arguments,
        // because `<Item = Int>` is for associated type bindings, not type parameters.
        if self.is_ident()
            && let Some(Token {
                kind: TokenKind::Ident(name),
                ..
            }) = self.stream.peek()
            && name.as_str() == "dyn"
        {
            self.stream.advance();

            // E096: Check for empty dyn - `dyn` without any protocol bounds
            if matches!(
                self.stream.peek_kind(),
                Some(TokenKind::RParen)
                    | Some(TokenKind::Semicolon)
                    | Some(TokenKind::Comma)
                    | Some(TokenKind::RBrace)
                    | Some(TokenKind::RBracket)
                    | Some(TokenKind::Eq)
                    | None
            ) {
                return Err(ParseError::dyn_no_protocol(self.stream.make_span(start_pos)));
            }

            // Parse first protocol bound (just the path, without type arguments)
            let mut bounds = Vec::new();
            let bound_start = self.stream.position();

            // Negative bound: !Protocol
            if self.stream.consume(&TokenKind::Bang).is_some() {
                let path = self.parse_path()?;
                let span = self.stream.make_span(bound_start);
                bounds.push(TypeBound {
                    kind: TypeBoundKind::NegativeProtocol(path),
                    span,
                });
            } else {
                // Positive bound: just the protocol path (not consuming <...>)
                let path = self.parse_path()?;
                let span = self.stream.make_span(bound_start);
                bounds.push(TypeBound {
                    kind: TypeBoundKind::Protocol(path),
                    span,
                });
            }

            // Parse optional associated type bindings: <Item = Int, State = String>
            // Parse optional associated type bindings: `<Item = Int, State = String>`
            let bindings = if self.stream.check(&TokenKind::Lt) {
                self.stream.advance(); // consume '<'
                let bindings = self.parse_type_bindings()?;
                self.expect_gt()?; // expect '>' (handles >> splitting for nested generics)
                Maybe::Some(bindings)
            } else {
                Maybe::None
            };

            // Continue parsing additional bounds after type bindings: + Display + Debug
            while self.stream.consume(&TokenKind::Plus).is_some() {
                let bound_start = self.stream.position();

                // Negative bound: !Protocol
                if self.stream.consume(&TokenKind::Bang).is_some() {
                    let path = self.parse_path()?;
                    let span = self.stream.make_span(bound_start);
                    bounds.push(TypeBound {
                        kind: TypeBoundKind::NegativeProtocol(path),
                        span,
                    });
                } else {
                    // Positive bound: Protocol
                    let path = self.parse_path()?;
                    let span = self.stream.make_span(bound_start);
                    bounds.push(TypeBound {
                        kind: TypeBoundKind::Protocol(path),
                        span,
                    });
                }
            }

            let span = self.stream.make_span(start_pos);

            return Ok(Type::new(
                TypeKind::DynProtocol {
                    bounds: bounds.into_iter().collect::<List<_>>(),
                    bindings,
                },
                span,
            ));
        }

        Err(ParseError::invalid_syntax(
            "expected protocol type",
            self.stream.current_span(),
        ))
    }

    /// Parse a kind expression for HKT kind annotations.
    ///
    /// Grammar:
    /// ```ebnf
    /// kind_expr = 'Type' , [ '->' , kind_expr ]
    ///           | '(' , kind_expr , ')' ;
    /// ```
    ///
    /// Examples:
    /// - `Type`              → `KindAnnotation::Type`
    /// - `Type -> Type`      → `Arrow(Type, Type)`
    /// - `Type -> Type -> Type` → `Arrow(Type, Arrow(Type, Type))`
    ///
    /// Called after the `:` in `F: Type -> Type`.
    /// The caller is responsible for having already confirmed that
    /// `TokenKind::Type` is next in the stream.
    fn parse_kind_annotation(&mut self) -> ParseResult<KindAnnotation> {
        let start_pos = self.stream.position();

        // Parenthesised kind: '(' kind_expr ')'
        if self.stream.consume(&TokenKind::LParen).is_some() {
            let inner = self.parse_kind_annotation()?;
            // Pre-capture span before the mutable `expect` call to satisfy the borrow checker.
            let paren_span = self.stream.make_span(start_pos);
            self.stream.expect(TokenKind::RParen).map_err(|_| {
                ParseError::invalid_syntax(
                    "expected `)` to close parenthesised kind expression",
                    paren_span,
                )
            })?;
            // After closing paren, check for arrow continuation
            return if self.stream.consume(&TokenKind::RArrow).is_some() {
                let rhs = self.parse_kind_annotation()?;
                Ok(KindAnnotation::Arrow(Box::new(inner), Box::new(rhs)))
            } else {
                Ok(inner)
            };
        }

        // Base kind: 'Type' — pre-capture span before the mutable `expect` call.
        let type_span = self.stream.make_span(start_pos);
        self.stream.expect(TokenKind::Type).map_err(|_| {
            ParseError::invalid_syntax(
                "expected `Type` in kind annotation (e.g. `F: Type -> Type`)",
                type_span,
            )
        })?;

        // Check for arrow: '->' means this is an arrow kind
        if self.stream.consume(&TokenKind::RArrow).is_some() {
            let rhs = self.parse_kind_annotation()?;
            Ok(KindAnnotation::Arrow(
                Box::new(KindAnnotation::Type),
                Box::new(rhs),
            ))
        } else {
            Ok(KindAnnotation::Type)
        }
    }
}
