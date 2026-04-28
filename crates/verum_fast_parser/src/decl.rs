//! Declaration parser for Verum using hand-written recursive descent.
//!
//! This module implements parsing for top-level items:
//! - Functions
//! - Types (records, variants, newtypes, aliases)
//! - Protocols
//! - Implementations
//! - Modules, constants, statics, imports
//!
//! ## Migration Status
//!
//! This module has been fully migrated to hand-written recursive descent parser.
//! It provides:
//! - API: `RecursiveParser::parse_module()`, `RecursiveParser::parse_item()`

use verum_ast::attr::AttributeTarget;
use verum_ast::bitfield::{BitSpec, BitWidth};
use verum_ast::decl::{ViewConstructor, ViewDecl};
use verum_ast::ffi::{
    CallingConvention, ErrorProtocol, FFIBoundary, FFIFunction, FFISignature, MemoryEffects,
    Ownership,
};
use verum_ast::{
    Expr, ExprKind, Ident, Item, ItemKind, LiteralKind, Span, Spanned, Type, TypeKind,
    decl::*,
    ty::{GenericParam, Path, PathSegment, WhereClause, WherePredicate, WherePredicateKind},
};
use verum_common::{Heap, List, Maybe, Text};
use verum_lexer::{Token, TokenKind};

use crate::error::{ErrorCode, ParseError, ParseErrorKind};
use crate::parser::{ParseResult, RecursiveParser};

// ============================================================================
// Hand-written Recursive Descent Parser Implementation
// ============================================================================

impl<'a> RecursiveParser<'a> {
    /// Parse a complete module (list of top-level items).
    ///
    /// When [`Self::script_mode`] is on, top-level statements
    /// (let-bindings, expression-statements, defer, …) are also
    /// accepted and folded into a single synthesised
    /// `__verum_script_main` `FunctionDecl` (P1.2). The function
    /// is appended after the regular items so source order
    /// `decl ; stmt ; decl ; stmt` survives unchanged.
    pub fn parse_module(&mut self) -> ParseResult<Vec<Item>> {
        // MEMORY OPTIMIZATION: Pre-allocate reasonable capacity based on typical module sizes
        // Most modules have 10-100 items, 64 is a good starting point
        let mut items = Vec::with_capacity(64);
        let mut script_stmts: Vec<verum_ast::Stmt> = Vec::new();
        let script_start = self.stream.current_span();

        while !self.stream.at_end() && self.tick() {
            let pos_before = self.stream.position();

            // Clear pending flags between items to ensure clean state
            self.pending_gt = false;
            self.pending_star = false;
            self.pending_ampersand = false;

            // Script-mode short-circuit: if the current token is an
            // unmistakable statement-starter (let / defer / errdefer
            // / provide), parse it as a statement directly so the
            // item-vs-statement disambiguator never has to recover
            // from a parse failure. Item-keywords (fn / type / const
            // / mount / static / implement / context / extern /
            // pure / async / meta) keep flowing through `parse_item`
            // which is also the path `parse_stmt` itself uses for
            // local items inside blocks.
            if self.script_mode && self.is_script_stmt_starter() {
                match self.parse_stmt() {
                    Ok(stmt) => {
                        script_stmts.push(stmt);
                        if self.is_aborted() { break; }
                        continue;
                    }
                    Err(e) => {
                        self.error(e);
                        self.synchronize();
                        if self.stream.position() == pos_before && !self.stream.at_end() {
                            self.stream.advance();
                        }
                        if self.is_aborted() { break; }
                        continue;
                    }
                }
            }

            match self.parse_item() {
                Ok(item) => {
                    items.push(item);
                }
                Err(e) => {
                    // In script mode, retry as a statement before
                    // surfacing the item-parse error — this catches
                    // expression-statements (`print(x);`,
                    // `do_thing();`) that can't be discriminated by
                    // a single look-ahead token.
                    if self.script_mode && self.stream.position() == pos_before {
                        if let Ok(stmt) = self.parse_stmt() {
                            script_stmts.push(stmt);
                            if self.is_aborted() { break; }
                            continue;
                        }
                    }

                    self.error(e);
                    self.synchronize();

                    // Safety: If synchronize() didn't advance (e.g., stuck on RBrace),
                    // we must advance at least one token to prevent infinite loop
                    if self.stream.position() == pos_before && !self.stream.at_end() {
                        self.stream.advance();
                    }
                }
            }

            // Check if aborted - exit early
            if self.is_aborted() {
                break;
            }
        }

        // If aborted due to safety limit, return error immediately
        if self.is_aborted() {
            return Err(ParseError::invalid_syntax(
                "parsing aborted due to safety limit (possible infinite loop)",
                Span::new(0, 0, self.file_id),
            ));
        }

        // Script mode: if any top-level statements were collected,
        // synthesise the `__verum_script_main` wrapper now so the
        // pipeline downstream sees a regular fn-decl item.
        if self.script_mode && !script_stmts.is_empty() {
            items.push(self.synthesize_script_main(script_stmts, script_start));
        }

        // Root fix for Issue #5 (parser error duplication):
        //
        // Return `Ok(items)` even when `self.errors` is non-empty; the
        // caller (`parse_module_internal` in lib.rs) already checks
        // `parser.errors.is_empty()` on the Ok-path and converts a
        // populated error list into `Err(errors)` there. Previously we
        // *also* returned `Err(self.errors.first().clone())` here,
        // which the caller then re-appended to `parser.errors` in its
        // Err-arm — duplicating the very first error across the final
        // error list. Concretely, three `assert!(...)` lines produced
        // four diagnostics: the original three plus a second copy of
        // the first. With this change, the caller's pre-existing
        // empty-check is the single source of truth for promoting the
        // parse to a failure, and no diagnostic is emitted twice.
        Ok(items)
    }

    /// Parse any top-level item.
    pub fn parse_item(&mut self) -> ParseResult<Item> {
        // Guard against deeply nested module { module { ... } } causing stack overflow
        self.enter_recursion()?;
        let result = self.parse_item_inner();
        self.exit_recursion();
        result
    }

    /// Script-mode discriminator: does the current token unambiguously
    /// begin a statement (rather than an item)? The four keywords
    /// `let` / `defer` / `errdefer` / `provide` are the unambiguous
    /// starters — every other case (item-keywords, expression
    /// statements, attribute-prefixed items / stmts) falls through
    /// to the `parse_item` → fallback `parse_stmt` ladder.
    fn is_script_stmt_starter(&self) -> bool {
        matches!(
            self.stream.peek_kind(),
            Some(TokenKind::Let)
                | Some(TokenKind::Defer)
                | Some(TokenKind::Errdefer)
                | Some(TokenKind::Provide)
        )
    }

    /// Synthesise the `__verum_script_main` wrapper that holds every
    /// top-level statement collected during script-mode parsing.
    ///
    /// The wrapper is a regular private `FunctionDecl` so all
    /// downstream passes (resolver, type-check, codegen) treat it
    /// uniformly. The compiler entry-detection pass recognises the
    /// well-known name `__verum_script_main` against
    /// `Module::is_script()` and uses it as the script's entry
    /// point (P1.3).
    ///
    /// **Tail-expression semantics.** Following the standard
    /// Verum / Rust block-as-expression rule, when the last collected
    /// statement is an expression-statement *without* a trailing
    /// semicolon, lift it into the wrapper block's tail-expression slot
    /// so its value becomes the wrapper's return value. The interpreter's
    /// exit-code propagation (`pipeline::propagate_main_exit_code`) then
    /// surfaces an `Int` tail value as the process exit status, making
    /// scripts like `print("done"); 42` produce exit-code 42 — natural
    /// and Unix-friendly without an explicit `exit()` call. Statements
    /// with `;` keep their void semantics (their value is discarded).
    fn synthesize_script_main(
        &self,
        mut stmts: Vec<verum_ast::Stmt>,
        first_stmt_span: Span,
    ) -> Item {
        // Span the wrapper across every statement we collected.
        let last_span = stmts
            .last()
            .map(|s| s.span())
            .unwrap_or(first_stmt_span);
        let span = Span::new(first_stmt_span.start, last_span.end, self.file_id);

        // Tail-expression lift: if the final collected statement is an
        // unsemicoloned expression-stmt, move its expression into the
        // block's `expr` slot so the wrapper returns its value.
        let mut tail_expr: Maybe<verum_common::Heap<verum_ast::Expr>> = Maybe::None;
        if matches!(
            stmts.last().map(|s| &s.kind),
            Some(verum_ast::StmtKind::Expr { has_semi: false, .. })
        ) {
            if let Some(last) = stmts.pop() {
                if let verum_ast::StmtKind::Expr { expr, .. } = last.kind {
                    tail_expr = Maybe::Some(verum_common::Heap::new(expr));
                }
            }
        }

        let body = verum_ast::Block {
            stmts: List::from(stmts),
            expr: tail_expr,
            span,
        };
        let func = FunctionDecl {
            visibility: Visibility::Private,
            is_async: false,
            is_meta: false,
            stage_level: 0,
            is_pure: false,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,
            extern_abi: Maybe::None,
            is_variadic: false,
            name: Ident::new(Text::from("__verum_script_main"), span),
            generics: List::new(),
            params: List::new(),
            return_type: Maybe::None,
            throws_clause: Maybe::None,
            std_attr: Maybe::None,
            contexts: List::new(),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            attributes: List::new(),
            body: Maybe::Some(verum_ast::decl::FunctionBody::Block(body)),
            span,
        };
        Item::new(ItemKind::Function(func), span)
    }

    fn parse_item_inner(&mut self) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // Try each item kind in order
        // Note: Order matters for disambiguation

        // Parse attributes first (common to many items)
        let attrs = self.parse_attributes()?;

        // FFI boundary (can have visibility modifiers and cfg attributes)
        // Handles: ffi, pub ffi, internal ffi, protected ffi
        if self.stream.check(&TokenKind::Ffi)
            || (matches!(
                self.stream.peek_kind(),
                Some(TokenKind::Pub) | Some(TokenKind::Internal) | Some(TokenKind::Protected)
            ) && matches!(
                self.stream.peek_nth(1).map(|t| &t.kind),
                Some(TokenKind::Ffi)
            ))
        {
            return self.parse_ffi_boundary(attrs);
        }

        let vis = self.parse_visibility()?;

        // Check for item keywords
        match self.stream.peek_kind() {
            // E033: Invalid visibility modifiers - 'private' is not a visibility in Verum
            // (items are private by default, no keyword needed)
            Some(TokenKind::Private) => {
                let span = self.stream.current_span();
                Err(ParseError::invalid_fn_visibility(
                    "'private' is not a valid visibility modifier; items are private by default",
                    span,
                ))
            }
            // E033: 'export' is not a visibility modifier in Verum (use 'pub' instead)
            Some(TokenKind::Ident(name)) if name.as_str() == "export" => {
                let span = self.stream.current_span();
                Err(ParseError::invalid_fn_visibility(
                    "'export' is not a valid visibility modifier; use 'pub' instead",
                    span,
                ))
            }
            // E034: Duplicate visibility modifier (e.g., pub pub fn)
            Some(TokenKind::Pub) | Some(TokenKind::Public) => {
                let span = self.stream.current_span();
                Err(ParseError::duplicate_fn_modifier("pub", span))
            }
            Some(TokenKind::Fn) => self.parse_function(attrs, vis),
            Some(TokenKind::Unsafe) => {
                // Could be: unsafe fn, unsafe implement, or unsafe type
                // Grammar: impl_block = [ attribute ] , [ 'unsafe' ] , 'implement' , ...
                // Grammar: function_modifiers = [ 'pure' ] , [ 'meta' ] , [ 'async' ] , [ 'unsafe' ]
                match self.stream.peek_nth(1).map(|t| &t.kind) {
                    Some(TokenKind::Implement) => {
                        // unsafe implement Protocol for Type { }
                        self.stream.advance(); // consume 'unsafe'
                        self.parse_impl_with_unsafe(attrs, true)
                    }
                    Some(TokenKind::Type) => {
                        // unsafe type TrustedLen is protocol { ... }
                        self.stream.advance(); // consume 'unsafe'
                        self.parse_type_decl(attrs, vis)
                    }
                    _ => {
                        // unsafe fn at module level
                        self.parse_function(attrs, vis)
                    }
                }
            }
            Some(TokenKind::Pure) => {
                // pure fn, pure meta fn, pure async fn, pure meta async fn
                // Grammar v2.12: function_modifiers = [ 'pure' ] , [ 'meta' ] , [ 'async' ] , [ 'unsafe' ]
                self.parse_function(attrs, vis)
            }
            Some(TokenKind::Extern) => {
                // extern fn, extern "ABI" fn, or extern { ... } block
                // Check if this is an extern block or extern function
                // extern { ... } or extern "ABI" { ... }
                let is_block = {
                    // Peek ahead to determine if this is a block
                    // extern { -> block
                    // extern "ABI" { -> block
                    // extern fn -> function
                    // extern "ABI" fn -> function
                    match self.stream.peek_nth(1).map(|t| &t.kind) {
                        Some(TokenKind::LBrace) => true,
                        Some(TokenKind::Text(_)) => {
                            // extern "ABI" - check if followed by { or fn
                            matches!(
                                self.stream.peek_nth(2).map(|t| &t.kind),
                                Some(TokenKind::LBrace)
                            )
                        }
                        _ => false,
                    }
                };
                if is_block {
                    self.parse_extern_block(attrs)
                } else {
                    self.parse_function(attrs, vis)
                }
            }
            Some(TokenKind::Cofix) => {
                // `cofix fn name(...) -> Stream<T> { .obs => expr, ... }` is a
                // coinductive fixpoint function. Grammar:
                //   function_modifiers = [ 'pure' ] , [ meta_modifier ] ,
                //                        [ 'async' ] , [ 'cofix' ] ,
                //                        [ 'unsafe' ] | epsilon ;
                // This dispatcher handles the plain `cofix fn ...` case when
                // no earlier modifier (async, meta, pure) consumed it. The
                // inner modifier-consumer in `parse_function` will accept the
                // `cofix` token on its own.
                self.parse_function(attrs, vis)
            }
            Some(TokenKind::Async) => {
                // Could be async fn, async unsafe fn, async context, etc.
                // Grammar: function_modifiers = [ 'pure' ] , [ meta_modifier ] , [ 'async' ] , [ 'unsafe' ]
                // After async, valid tokens are: fn, unsafe, cofix, extern, or context
                match self.stream.peek_nth(1).map(|t| &t.kind) {
                    Some(TokenKind::Fn) | Some(TokenKind::Unsafe) | Some(TokenKind::Cofix)
                    | Some(TokenKind::Extern) => self.parse_function(attrs, vis),
                    Some(TokenKind::Context) => {
                        // async context Name
                        self.stream.advance(); // consume 'async'
                        self.parse_context(vis, true)
                    }
                    // E034: Duplicate async modifier
                    Some(TokenKind::Async) => {
                        self.stream.advance(); // consume first 'async'
                        Err(ParseError::duplicate_fn_modifier("async", self.stream.current_span()))
                    }
                    _ => Err(ParseError::invalid_syntax(
                        "expected 'fn', 'unsafe', or 'context' after async keyword",
                        self.stream.current_span(),
                    )),
                }
            }
            Some(TokenKind::Meta) => {
                // Could be meta fn, meta(N) fn, or standalone meta declaration
                // Grammar: meta_modifier = 'meta' , [ '(' , stage_level , ')' ]
                // Staged meta examples:
                //   meta fn derive_eq() { }      // Stage 1 (default)
                //   meta(1) fn derive_eq() { }   // Stage 1 (explicit)
                //   meta(2) fn create_derives()  // Stage 2: generates meta functions
                //   meta async fn ...            // Stage 1 async
                match self.stream.peek_nth(1).map(|t| &t.kind) {
                    // meta fn, meta async fn
                    Some(TokenKind::Fn) | Some(TokenKind::Async) => self.parse_function(attrs, vis),
                    // meta(N) fn - staged metaprogramming with explicit level
                    Some(TokenKind::LParen) => self.parse_function(attrs, vis),
                    // Standalone meta declaration (not a function)
                    _ => self.parse_meta(vis),
                }
            }
            Some(TokenKind::Type) => self.parse_type_decl(attrs, vis),
            Some(TokenKind::Protocol) => self.parse_protocol_decl_with_context(attrs, vis, false),
            Some(TokenKind::Implement) => self.parse_impl_with_unsafe(attrs, false),
            Some(TokenKind::Context) => {
                // Could be: context Name, context async Name, context group Name,
                // context protocol Name, or context type Name
                // Check what follows the context keyword
                match self.stream.peek_nth(1).map(|t| &t.kind) {
                    Some(TokenKind::Ident(name)) if name.as_str() == "group" => {
                        // context group Name
                        self.parse_context_group(vis)
                    }
                    Some(TokenKind::Protocol) => {
                        // context protocol Name { ... }
                        self.stream.advance(); // consume 'context'
                        self.parse_protocol_decl_with_context(attrs, vis, true)
                    }
                    Some(TokenKind::Type) => {
                        // context type Name is protocol { ... }
                        self.stream.advance(); // consume 'context'
                        self.parse_type_decl_with_context(attrs, vis)
                    }
                    Some(TokenKind::Async) => {
                        // context async Name
                        self.stream.advance(); // consume 'context'
                        self.stream.advance(); // consume 'async'
                        self.parse_context(vis, true)
                    }
                    _ => {
                        // context Name
                        self.parse_context(vis, false)
                    }
                }
            }
            Some(TokenKind::Const) => self.parse_const(vis),
            // Allow `let` at item level as equivalent to `const`
            // Handles: `public let NAME: Type = value;`
            Some(TokenKind::Let) => self.parse_const(vis),
            Some(TokenKind::Static) => self.parse_static(attrs, vis),
            Some(TokenKind::Module) => self.parse_module_decl(attrs, vis),
            Some(TokenKind::Mount) => self.parse_mount(attrs.into(), vis),
            // `link` is treated as a synonym for `mount` in import contexts
            Some(TokenKind::Link) => self.parse_mount(attrs.into(), vis),
            Some(TokenKind::Using) => {
                // Check if this is a context group alias: using Name = [...]
                // vs. a function using clause (which shouldn't appear at top-level)
                if let Some(Token {
                    kind: TokenKind::Ident(_),
                    ..
                }) = self.stream.peek_nth(1)
                    && let Some(Token {
                        kind: TokenKind::Eq,
                        ..
                    }) = self.stream.peek_nth(2)
                {
                    return self.parse_context_group_alias(vis);
                }
                // Module-level context requirement: using [Context1, Context2]
                // Parsed as a context group with implicit name "__module_contexts__"
                if let Some(Token {
                    kind: TokenKind::LBracket,
                    ..
                }) = self.stream.peek_nth(1)
                {
                    return self.parse_module_context_requirement(vis);
                }
                // Otherwise, this is an error (using at top-level without alias syntax)
                let span = self.stream.current_span();
                Err(ParseError::invalid_syntax(
                    "unexpected 'using' at top-level; did you mean 'using Name = [contexts...]'?",
                    span,
                ))
            }
            Some(TokenKind::Ident(name)) if name.as_str() == "predicate" => {
                self.parse_predicate(vis)
            }
            Some(TokenKind::Ident(name)) if name.as_str() == "tactic" => {
                self.parse_tactic_decl(vis)
            }
            // `active pattern` syntax: `active pattern Name(params) -> Type = expr;`
            // The `active` keyword prefix is optional syntactic sugar
            Some(TokenKind::Ident(name)) if name.as_str() == "active" && matches!(self.stream.peek_nth_kind(1), Some(&TokenKind::ActivePattern)) => {
                self.stream.advance(); // consume `active`
                self.parse_pattern_decl(attrs, vis)
            }
            // Ghost declarations: ghost type, ghost fn, ghost axiom, ghost lemma, etc.
            // The `ghost` keyword prefix indicates verification-only constructs.
            Some(TokenKind::Ident(name)) if name.as_str() == "ghost" => {
                self.stream.advance(); // consume `ghost`
                // Now parse the actual declaration and wrap it
                // Ghost items parse normally, the ghost prefix is informational for verification
                match self.stream.peek_kind() {
                    Some(TokenKind::Type) => self.parse_type_decl(attrs, vis),
                    Some(TokenKind::Fn) => self.parse_function(attrs, vis),
                    Some(TokenKind::Axiom) => self.parse_axiom(attrs, vis),
                    Some(TokenKind::Lemma) => self.parse_lemma(attrs, vis),
                    Some(TokenKind::Theorem) => self.parse_theorem(attrs, vis),
                    Some(TokenKind::Const) => self.parse_const(vis),
                    Some(TokenKind::Static) => self.parse_static(attrs, vis),
                    _ => {
                        // Unknown ghost item — produce an error
                        let span = self.stream.current_span();
                        Err(ParseError::invalid_syntax(
                            "expected 'type', 'fn', 'axiom', 'lemma', 'theorem', 'const', or 'static' after 'ghost'",
                            span,
                        ))
                    }
                }
            }
            // Formal proofs: theorem, axiom, lemma, corollary declarations
            Some(TokenKind::Theorem) => self.parse_theorem(attrs, vis),
            Some(TokenKind::Axiom) => self.parse_axiom(attrs, vis),
            Some(TokenKind::Lemma) => self.parse_lemma(attrs, vis),
            Some(TokenKind::Corollary) => self.parse_corollary(attrs, vis),
            // Proof declarations: `proof name(params): proposition { proof_body }`
            // Treated as theorem-like declarations
            Some(TokenKind::Proof) => self.parse_proof_decl(attrs, vis),
            // View declarations: alternative pattern interfaces for dependent type matching
            Some(TokenKind::View) => self.parse_view_decl(attrs, vis),
            // Active pattern declarations: user-defined pattern-matching decomposition functions
            Some(TokenKind::ActivePattern) => self.parse_pattern_decl(attrs, vis),
            // Context layer declarations: composable context bundles
            Some(TokenKind::Layer) => self.parse_layer(attrs, vis),
            // E011: Unmatched closing brace at module level
            Some(TokenKind::RBrace) => {
                let span = self.stream.current_span();
                Err(ParseError::stmt_unclosed_block(span)
                    .with_help("unexpected '}' at module level; check for unmatched braces"))
            }
            // ================================================================
            // Rust syntax detection: provide helpful migration messages
            // ================================================================
            Some(TokenKind::Ident(name)) if name.as_str() == "struct" => {
                let span = self.stream.current_span();
                Err(ParseError::rust_keyword_used(
                    "struct",
                    "type Name is { field: Type, ... }",
                    span,
                ))
            }
            Some(TokenKind::Ident(name)) if name.as_str() == "enum" => {
                let span = self.stream.current_span();
                Err(ParseError::rust_keyword_used(
                    "enum",
                    "type Name is A | B(T) | C { field: Type }",
                    span,
                ))
            }
            Some(TokenKind::Ident(name)) if name.as_str() == "trait" => {
                let span = self.stream.current_span();
                Err(ParseError::rust_keyword_used(
                    "trait",
                    "type Name is protocol { ... }",
                    span,
                ))
            }
            Some(TokenKind::Ident(name)) if name.as_str() == "impl" => {
                let span = self.stream.current_span();
                Err(ParseError::rust_keyword_used(
                    "impl",
                    "implement",
                    span,
                ))
            }
            Some(TokenKind::Ident(name)) if name.as_str() == "use" => {
                let span = self.stream.current_span();
                Err(ParseError::rust_keyword_used(
                    "use",
                    "mount",
                    span,
                ))
            }
            Some(TokenKind::Ident(name)) if name.as_str() == "mod" => {
                let span = self.stream.current_span();
                Err(ParseError::rust_keyword_used(
                    "mod",
                    "module",
                    span,
                ))
            }
            _ => {
                let span = self.stream.current_span();
                Err(ParseError::invalid_syntax(
                    "expected item (fn, type, implement, const, static, module, mount, theorem, axiom, lemma, etc.)",
                    span,
                ))
            }
        }
    }

    /// Parse a function declaration.
    /// Grammar v2.12: function_modifiers = [ 'pure' ] , [ 'meta' ] , [ 'async' ] , [ 'unsafe' ]
    fn parse_function(&mut self, attrs: Vec<Attribute>, vis: Visibility) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // Optional pure keyword (explicit purity declaration)
        // Spec: grammar/verum.ebnf v2.12 - function_modifiers
        let is_pure = self.stream.consume(&TokenKind::Pure).is_some();
        // E034: Check for duplicate pure modifier
        if is_pure && self.stream.check(&TokenKind::Pure) {
            return Err(ParseError::duplicate_fn_modifier("pure", self.stream.current_span()));
        }

        // Optional meta keyword with optional stage level: meta or meta(N)
        // Uses parse_meta_modifier helper for staged metaprogramming support.
        let (is_meta, stage_level) = self.parse_meta_modifier()?;

        // Optional async keyword
        let is_async = self.stream.consume(&TokenKind::Async).is_some();
        // E034: Check for duplicate async modifier
        if is_async && self.stream.check(&TokenKind::Async) {
            return Err(ParseError::duplicate_fn_modifier("async", self.stream.current_span()));
        }

        // Optional cofix keyword (coinductive fixpoint)
        let is_cofix = self.stream.consume(&TokenKind::Cofix).is_some();

        // Optional unsafe keyword
        // Unsafe functions bypass Verum safety guarantees (required for FFI via C ABI)
        let is_unsafe = self.stream.consume(&TokenKind::Unsafe).is_some();
        // E034: Check for duplicate unsafe modifier
        if is_unsafe && self.stream.check(&TokenKind::Unsafe) {
            return Err(ParseError::duplicate_fn_modifier("unsafe", self.stream.current_span()));
        }

        // Optional extern keyword with optional ABI
        // Syntax: extern fn foo() or extern "C" fn bar()
        let extern_abi = if self.stream.consume(&TokenKind::Extern).is_some() {
            // Check for optional ABI string literal
            if let Some(TokenKind::Text(abi)) = self.stream.peek_kind() {
                let abi_text = abi.clone();
                self.stream.advance();
                Maybe::Some(abi_text)
            } else {
                // No ABI specified, use default (None means default platform ABI)
                Maybe::Some(Text::from(""))
            }
        } else {
            Maybe::None
        };

        // fn keyword
        self.stream.expect(TokenKind::Fn)?;

        // Optional '*' for generator functions (fn*)
        // Spec: grammar/verum.ebnf v2.10 - fn_keyword = 'fn' , [ '*' ]
        let is_generator = self.stream.consume(&TokenKind::Star).is_some();

        // Function name - allow contextual keywords as function names (e.g., `show` which is a proof keyword)
        // E030: Missing function name - check if next token is '(' instead of identifier
        let name = match self.stream.peek_kind() {
            Some(TokenKind::LParen) => {
                // fn () { } - missing function name
                return Err(ParseError::missing_fn_name(self.stream.current_span()));
            }
            Some(TokenKind::Lt) => {
                // fn <T>() { } - missing function name before generic params
                return Err(ParseError::missing_fn_name(self.stream.current_span()));
            }
            _ => self.consume_ident_or_keyword()?,
        };
        let name_span = self.stream.current_span();

        // Generic parameters: <T, U>
        let generics: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?
        } else {
            List::new()
        };

        // Parameters: (x: Int, y: Float) or variadic (x: Int, ...)
        // E031: Missing function parameter list - check if '(' is missing
        if !self.stream.check(&TokenKind::LParen) {
            return Err(ParseError::missing_fn_params(self.stream.current_span()));
        }
        let (params, is_variadic) = self.parse_function_params_with_variadic()?;

        // Throws clause: throws(ErrorType | OtherError)
        // Spec: grammar/verum.ebnf v2.8 - Section 2.4 throws_clause
        // Example: fn parse(input: Text) throws(ParseError | ValidationError) -> AST
        let throws_clause = self.parse_throws_clause()?;

        // Pre-return-type `using [Ctx]` — the alternate ordering
        // `fn foo() using [Ctx] -> Int { … }` appears throughout the
        // L0/vbc/context VCS specs. The canonical spelling (below)
        // puts `using` after the return type; accepting the pre-
        // return position too keeps both stylistic conventions valid
        // without forcing the stdlib or tests to pick a side.
        let mut contexts: Vec<ContextRequirement> = Vec::new();
        if self.stream.check(&TokenKind::Using) && !contexts.is_empty() {
            // Already populated — skip (defensive; contexts starts empty).
        } else if self.stream.consume(&TokenKind::Using).is_some() {
            contexts = self.parse_using_contexts()?;
        }

        // Return type: -> Type
        // Use parse_type_with_lookahead to support refinements like `-> Int{>= 0}`
        // but avoid consuming the function body `{`
        let return_type = if self.stream.consume(&TokenKind::RArrow).is_some() {
            // E037: Invalid return type - check for common errors
            match self.stream.peek_kind() {
                // Missing type after arrow: `fn foo() -> {}`
                Some(TokenKind::LBrace) => {
                    return Err(ParseError::invalid_return_type(
                        "missing return type after '->'",
                        self.stream.current_span(),
                    ));
                }
                // Double arrow: `fn foo() -> -> Int`
                Some(TokenKind::RArrow) => {
                    return Err(ParseError::invalid_return_type(
                        "duplicate '->' in return type",
                        self.stream.current_span(),
                    ));
                }
                // Literal instead of type: `fn foo() -> 123`
                Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_)) => {
                    return Err(ParseError::invalid_return_type(
                        "expected type, found literal",
                        self.stream.current_span(),
                    ));
                }
                _ => {}
            }
            Maybe::Some(self.parse_type_with_lookahead()?)
        } else {
            Maybe::None
        };

        // Context requirements AFTER return type (CANONICAL FORM)
        // Context requirements: `using [Ctx1, Ctx2]` or `using Ctx` after return type (canonical form)
        // Format: fn foo() -> Type using [Context1, Context2]
        // Format: fn foo() using [Context] -- when return type is unit
        // Supports both: [Database, Logger] and using Database / using [...]
        if contexts.is_empty() {
            if self.stream.check(&TokenKind::LBracket) {
                contexts = self.parse_context_requirements()?;
            } else if self.stream.consume(&TokenKind::Using).is_some() {
                contexts = self.parse_using_contexts()?;
            }
        }

        // M205: Check for duplicate using clause
        // Grammar: function_def = ... , [ context_clause ] , ... ; (at most one)
        if self.stream.check(&TokenKind::Using) {
            return Err(ParseError::meta_duplicate_using(self.stream.current_span()));
        }

        // Where clause: where T: Ord or where type T: Ord or where meta N > 0
        // OR ensures clause: where ensures EXPR (postcondition syntax)
        // GRAMMAR:
        //   ensures_clause = 'where' , ensures_item , { ',' , ensures_item } ;
        //   ensures_item = 'ensures' , expression ;
        // Parse where clauses supporting two forms:
        // 1. Single where clause with mixed predicates: where T: Clone, meta stage > 0
        // 2. Separate where clauses: where T: Clone where meta stage > 0
        // GRAMMAR: function_def = ... , [ generic_where_clause ] , [ meta_where_clause ] , function_body ;
        // Order: generic FIRST, meta SECOND (E038 if meta comes first)
        let (generic_where, meta_where) = {
            let mut generic_preds: Vec<WherePredicate> = Vec::new();
            let mut meta_preds: Vec<WherePredicate> = Vec::new();
            let mut first_generic_span: Option<Span> = None;
            let mut first_meta_span: Option<Span> = None;
            let mut had_generic = false;
            let mut had_meta = false;

            // Parse all where clauses
            while self.stream.peek_kind() == Some(&TokenKind::Where) {
                // Skip `where ensures` (postcondition) - handled separately
                if self.stream.peek_nth(1).map(|t| &t.kind) == Some(&TokenKind::Ensures) {
                    break;
                }

                let clause_span = self.stream.current_span();
                let where_clause = self.parse_where_clause()?;

                // Separate this clause's predicates
                for pred in where_clause.predicates.iter() {
                    match &pred.kind {
                        WherePredicateKind::Meta { .. } => {
                            // E038: Check for wrong order - meta before generic
                            if !had_generic && had_meta {
                                // This is fine, consecutive meta predicates
                            } else if had_generic {
                                // meta after generic - allowed (correct order)
                            } else {
                                // meta FIRST - track it, will check if generic comes after
                                if first_meta_span.is_none() {
                                    first_meta_span = Some(pred.span);
                                }
                            }
                            had_meta = true;
                            meta_preds.push(pred.clone());
                        }
                        WherePredicateKind::Value { .. } | WherePredicateKind::Ensures { .. } => {
                            // Value/ensures predicates (bare expressions like `C > 0`)
                            // are compatible with meta predicates - treat as meta when
                            // they appear alongside meta predicates in the same clause
                            if had_meta {
                                meta_preds.push(pred.clone());
                            } else {
                                had_generic = true;
                                if first_generic_span.is_none() {
                                    first_generic_span = Some(pred.span);
                                }
                                generic_preds.push(pred.clone());
                            }
                        }
                        _ => {
                            // E038: Generic after meta is wrong order
                            if had_meta {
                                return Err(ParseError::where_clause_order(clause_span));
                            }
                            had_generic = true;
                            if first_generic_span.is_none() {
                                first_generic_span = Some(pred.span);
                            }
                            generic_preds.push(pred.clone());
                        }
                    }
                }
            }

            let generic_clause = if !generic_preds.is_empty() {
                let span = first_generic_span.unwrap_or_default();
                Maybe::Some(WhereClause {
                    predicates: generic_preds.into_iter().collect::<List<_>>(),
                    span,
                })
            } else {
                Maybe::None
            };

            let meta_clause = if !meta_preds.is_empty() {
                let span = first_meta_span.unwrap_or_default();
                Maybe::Some(WhereClause {
                    predicates: meta_preds.into_iter().collect::<List<_>>(),
                    span,
                })
            } else {
                Maybe::None
            };

            (generic_clause, meta_clause)
        };

        // Contract clauses: requires EXPR, ensures EXPR (repeatable)
        // Also supports: where ensures EXPR (postcondition syntax from grammar)
        // Also support contract literals: contract#"requires ..." / contract#"ensures ..."
        // Note: We use parse_expr_no_struct to prevent the { from the function body
        // being consumed as a struct literal in the contract expression
        let mut requires = Vec::new();
        let mut ensures = Vec::new();

        loop {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }

            // Handle @ghost prefix on contract clauses
            if self.stream.check(&TokenKind::At) {
                if let Some(TokenKind::Ident(name)) = self.stream.peek_nth_kind(1) {
                    if name.as_str() == "ghost" {
                        match self.stream.peek_nth_kind(2) {
                            // @ghost ensures/requires/invariant/decreases: skip @ghost prefix
                            Some(&TokenKind::Ensures) | Some(&TokenKind::Requires) | Some(&TokenKind::Invariant) | Some(&TokenKind::Decreases) => {
                                self.stream.advance(); // consume @
                                self.stream.advance(); // consume ghost
                                // Fall through to normal contract clause parsing
                            }
                            // @ghost(old_arr: Type = expr): ghost parameter clause
                            // Skip the entire @ghost(...) construct
                            Some(&TokenKind::LParen) => {
                                self.stream.advance(); // consume @
                                self.stream.advance(); // consume ghost
                                self.stream.advance(); // consume (
                                // Skip everything until matching )
                                let mut depth = 1u32;
                                while depth > 0 {
                                    match self.stream.peek_kind() {
                                        None => break,
                                        Some(TokenKind::LParen) => { depth += 1; self.stream.advance(); }
                                        Some(TokenKind::RParen) => { depth -= 1; self.stream.advance(); }
                                        _ => { self.stream.advance(); }
                                    }
                                }
                                continue; // Continue to next contract clause
                            }
                            _ => {}
                        }
                    }
                }
            }

            match self.stream.peek_kind() {
                Some(TokenKind::Requires) => {
                    self.stream.advance();
                    let expr = self.parse_expr_no_struct()?;
                    requires.push(expr);
                }
                Some(TokenKind::Ensures) => {
                    self.stream.advance();
                    let expr = self.parse_expr_no_struct()?;
                    ensures.push(expr);
                }
                Some(TokenKind::Decreases) => {
                    self.stream.advance();
                    // Parse decreases expression(s) - supports comma-separated for lexicographic ordering
                    // e.g., `decreases m, n` means lexicographic ordering on (m, n)
                    let _expr = self.parse_expr_no_struct()?;
                    // Consume additional comma-separated decreases expressions
                    while self.stream.check(&TokenKind::Comma) {
                        // Look ahead: if comma is followed by `ident :` it's a parameter, not another decreases expr
                        let is_param = matches!(
                            (self.stream.peek_nth_kind(1), self.stream.peek_nth_kind(2)),
                            (Some(&TokenKind::Ident(_)), Some(&TokenKind::Colon))
                        );
                        if is_param {
                            break;
                        }
                        self.stream.advance(); // consume comma
                        let _next_expr = self.parse_expr_no_struct()?;
                    }
                }
                // GRAMMAR: ensures_clause = 'where' , ensures_item , { ',' , ensures_item } ;
                // Handle `where ensures EXPR` postcondition syntax
                Some(TokenKind::Where) if self.stream.peek_nth(1).map(|t| &t.kind) == Some(&TokenKind::Ensures) => {
                    self.stream.advance(); // consume 'where'
                    // Now parse one or more ensures items (separated by comma)
                    loop {
                        if self.stream.consume(&TokenKind::Ensures).is_some() {
                            let expr = self.parse_expr_no_struct()?;
                            ensures.push(expr);
                            // Check for comma to continue
                            if self.stream.consume(&TokenKind::Comma).is_none() {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }
                Some(TokenKind::ContractLiteral(_)) => {
                    // Parse contract literal and add to the function
                    // Contract literals are treated as-is without further parsing
                    let expr = self.parse_expr_no_struct()?;
                    // Contract literals can be added to both requires and ensures
                    // The actual contract content will be processed later
                    // For now, we just store them as contract expressions
                    requires.push(expr);
                }
                _ => break,
            }
        }

        // Function body: { ... } or = expr;
        // Extern functions may have a body (exported) or just a declaration (imported)
        let body = if extern_abi.is_some() {
            // Extern functions WITH a body are "exported" functions
            // (equivalent to Rust's `extern "C" fn name() { body }`)
            // Extern functions WITHOUT a body (ending with ;) are "imported" FFI declarations
            if self.stream.check(&TokenKind::LBrace) {
                Maybe::Some(FunctionBody::Block(self.parse_block()?))
            } else if self.stream.consume(&TokenKind::Eq).is_some() {
                let expr = self.parse_expr()?;
                if !self.is_block_form_expr_for_fn(&expr) || self.stream.check(&TokenKind::Semicolon) {
                    self.stream.expect(TokenKind::Semicolon)?;
                }
                Maybe::Some(FunctionBody::Expr(expr))
            } else {
                self.stream.expect(TokenKind::Semicolon)?;
                Maybe::None
            }
        } else if self.stream.check(&TokenKind::LBrace) {
            // For cofix functions, check if the brace body is a copattern body
            // by looking ahead for the `.identifier =>` pattern.
            // Copattern body: `{ .obs => expr, ... }` (first token after `{` is `.`)
            if is_cofix && self.is_copattern_body_ahead() {
                let expr = self.parse_copattern_body()?;
                Maybe::Some(FunctionBody::Expr(expr))
            } else {
                Maybe::Some(FunctionBody::Block(self.parse_block()?))
            }
        } else if self.stream.consume(&TokenKind::Eq).is_some() {
            let expr = self.parse_expr()?;
            // For block-form expressions (match, if, block), semicolon is optional
            if !self.is_block_form_expr_for_fn(&expr) || self.stream.check(&TokenKind::Semicolon) {
                self.stream.expect(TokenKind::Semicolon)?;
            }
            Maybe::Some(FunctionBody::Expr(expr))
        } else if self.stream.consume(&TokenKind::Semicolon).is_some() {
            // Forward declaration without body
            // Valid for intrinsics, FFI bindings, or forward declarations
            Maybe::None
        } else {
            // E032: Missing function body
            return Err(ParseError::missing_fn_body(self.stream.current_span()));
        };

        // Extract @std and @transparent attributes if present
        let mut std_attr_opt = None;
        let mut is_transparent = false;
        let mut filtered_attrs = Vec::new();

        for attr in attrs.iter() {
            if attr.name.as_str() == "std" {
                let context_group: Maybe<Text> = match &attr.args {
                    Some(args) => match args.first() {
                        Some(first_arg) => match &first_arg.kind {
                            verum_ast::ExprKind::Path(path) => match path.segments.first() {
                                Some(seg) => match seg {
                                    verum_ast::PathSegment::Name(ident) => Some(ident.name.clone()),
                                    _ => None,
                                },
                                None => None,
                            },
                            _ => None,
                        },
                        None => None,
                    },
                    None => None,
                };

                std_attr_opt = Some(verum_ast::attr::StdAttr {
                    context_group,
                    span: attr.span,
                });
            } else if attr.name.as_str() == "transparent" {
                // @transparent attribute - disables hygienic macro expansion
                // Only meaningful for meta functions, but we set the flag regardless
                // and let the type checker validate proper usage
                is_transparent = true;
                // Don't filter out - keep in attributes for later phases
                filtered_attrs.push(attr.clone());
            } else {
                filtered_attrs.push(attr.clone());
            }
        }

        let span = self.stream.make_span(start_pos);
        Ok(Item::new(
            ItemKind::Function(FunctionDecl {
                visibility: vis,
                is_async,
                is_meta,
                stage_level,
                is_pure,
                is_generator,
                is_cofix,
                is_unsafe,
                is_transparent,
                extern_abi,
                is_variadic,
                name: Ident::new(name, name_span),
                generics,
                params: params.into_iter().collect(),
                return_type,
                throws_clause,
                std_attr: std_attr_opt,
                contexts: contexts.into_iter().collect(),
                generic_where_clause: generic_where,
                meta_where_clause: meta_where,
                requires: requires.into_iter().collect(),
                ensures: ensures.into_iter().collect(),
                attributes: filtered_attrs.into_iter().collect(),
                body,
                span,
            }),
            span,
        ))
    }

    /// Parse an extern block containing FFI function declarations.
    ///
    /// Grammar:
    /// ```ebnf
    /// extern_block = 'extern' , [ string_lit ] , '{' , { extern_fn } , '}' ;
    /// extern_fn = [ visibility ] , 'fn' , identifier , '(' , param_list , ')' , [ '->' , type ] , ';' ;
    /// ```
    ///
    /// Examples:
    /// ```verum
    /// extern "C" {
    ///     fn malloc(size: Int) -> &unsafe Byte;
    ///     fn free(ptr: &unsafe Byte);
    /// }
    /// ```
    fn parse_extern_block(&mut self, attrs: Vec<Attribute>) -> ParseResult<Item> {
        use verum_ast::decl::ExternBlockDecl;

        let start_span = self.stream.current_span();

        // Consume 'extern'
        self.stream.expect(TokenKind::Extern)?;

        // Optional ABI string: "C", "stdcall", etc.
        let abi = if let Some(TokenKind::Text(abi_text)) = self.stream.peek_kind() {
            let text = abi_text.clone();
            self.stream.advance();
            Maybe::Some(text)
        } else {
            Maybe::None
        };

        // Expect '{'
        self.stream.expect(TokenKind::LBrace)?;

        // Parse function declarations inside the block
        let mut functions = List::new();

        while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
            // Check for closing brace again after potential comment skipping
            if self.stream.check(&TokenKind::RBrace) {
                break;
            }

            // Skip const declarations inside extern blocks
            if self.stream.check(&TokenKind::Const) {
                while !self.stream.check(&TokenKind::Semicolon)
                    && !self.stream.check(&TokenKind::RBrace)
                    && !self.stream.at_end()
                {
                    self.stream.advance();
                }
                self.stream.consume(&TokenKind::Semicolon);
                continue;
            }
            // Type declarations are not allowed inside extern blocks
            if self.stream.check(&TokenKind::Type) {
                return Err(ParseError::invalid_syntax(
                    "type definitions are not allowed inside extern blocks; only function declarations are permitted",
                    self.stream.current_span(),
                ));
            }

            // Parse attributes (e.g., @link_name("..."), @no_mangle)
            let fn_attrs = self.parse_attributes()?;

            // Parse optional visibility
            let vis = self.parse_visibility()?;

            // Expect 'fn' keyword
            self.stream.expect(TokenKind::Fn)?;

            // Parse function name - allow keywords as names (e.g., `link`, `select`, `mount`)
            let name = self.consume_ident_or_any_keyword()?;
            let name_span = self.stream.current_span();

            // Parse generic parameters if present
            let generics: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
                self.parse_generic_params()?.into_iter().collect()
            } else {
                List::new()
            };

            // Parse parameters
            let (params, is_variadic) = self.parse_function_params_with_variadic()?;

            // Parse optional return type
            let return_type = if self.stream.consume(&TokenKind::RArrow).is_some() {
                Maybe::Some(self.parse_type()?)
            } else {
                Maybe::None
            };

            // Expect semicolon (extern functions have no body)
            self.stream.expect(TokenKind::Semicolon)?;

            let fn_span = start_span.merge(self.stream.current_span());

            // Create the function declaration with extern ABI
            let func = FunctionDecl {
                visibility: vis,
                is_async: false,
                is_meta: false,
                stage_level: 0,  // FFI functions are always runtime (stage 0)
                is_pure: false,
                is_generator: false,
                is_cofix: false,
                is_unsafe: false,
                is_transparent: false,  // FFI functions cannot be transparent
                extern_abi: abi.clone().or(Maybe::Some(Text::from("C"))),
                is_variadic,
                name: Ident::new(name, name_span),
                generics,
                params: params.into_iter().collect(),
                throws_clause: Maybe::None,
                return_type,
                contexts: List::new(),
                generic_where_clause: Maybe::None,
                meta_where_clause: Maybe::None,
                requires: List::new(),
                ensures: List::new(),
                attributes: fn_attrs.into_iter().collect(),
                body: Maybe::None,
                std_attr: Maybe::None,
                span: fn_span,
            };

            functions.push(func);
        }

        // Expect '}'
        self.stream.expect(TokenKind::RBrace)?;

        let span = start_span.merge(self.stream.current_span());

        Ok(Item::new(
            ItemKind::ExternBlock(ExternBlockDecl {
                abi,
                functions,
                attributes: attrs.into_iter().collect(),
                span,
            }),
            span,
        ))
    }

    /// Parse meta modifier with optional stage level for staged metaprogramming.
    ///
    /// # Syntax
    ///
    /// ```text
    /// meta_modifier = 'meta' [ '(' stage_level ')' ]
    /// stage_level   = integer_lit   (* Non-negative integer, 1 if omitted *)
    /// ```
    ///
    /// # Staged Metaprogramming Semantics
    ///
    /// Verum supports N-level staged metaprogramming:
    ///
    /// - **Stage 0**: Runtime execution (normal functions, no `meta` keyword)
    /// - **Stage 1**: Compile-time execution (`meta fn` or `meta(1) fn`)
    /// - **Stage N**: N-th level meta (`meta(N) fn` where N ≥ 2)
    ///
    /// # Stage Coherence Rule
    ///
    /// A Stage N function can only DIRECTLY generate Stage N-1 code.
    /// To generate lower-stage code, the output must contain meta functions
    /// that perform further generation.
    ///
    /// # Examples
    ///
    /// ```verum
    /// meta fn derive_eq() { ... }           // Stage 1: generates runtime code
    /// meta(1) fn derive_eq() { ... }        // Same as above (explicit)
    /// meta(2) fn derive_family() { ... }    // Stage 2: generates meta functions
    /// meta(3) fn dsl_compiler() { ... }     // Stage 3: generates Stage 2 code
    /// ```
    ///
    /// # Returns
    ///
    /// `(is_meta, stage_level)` where:
    /// - `is_meta = false, stage_level = 0` if no `meta` keyword
    /// - `is_meta = true, stage_level = 1` for plain `meta`
    /// - `is_meta = true, stage_level = N` for `meta(N)`
    fn parse_meta_modifier(&mut self) -> ParseResult<(bool, u32)> {
        if self.stream.consume(&TokenKind::Meta).is_some() {
            // Check for optional (N) stage level
            if self.stream.consume(&TokenKind::LParen).is_some() {
                // Parse stage level integer
                let level_span = self.stream.current_span();
                let level = match self.stream.peek_kind() {
                    Some(TokenKind::Integer(n)) => {
                        // Use as_u64() which properly handles the IntegerLiteral
                        let parsed = n.as_u64().and_then(|v| {
                            if v >= 1 && v <= u32::MAX as u64 {
                                Some(v as u32)
                            } else {
                                None
                            }
                        });
                        match parsed {
                            Some(v) => {
                                self.stream.advance();
                                v
                            }
                            None => {
                                // M006: Stage level is 0 or out of range
                                return Err(ParseError::meta_invalid_stage(
                                    level_span,
                                    "stage level must be a positive integer between 1 and 2^32-1",
                                ));
                            }
                        }
                    }
                    Some(TokenKind::Minus) => {
                        // M006: Negative stage level (e.g., meta(-1))
                        return Err(ParseError::meta_invalid_stage(
                            level_span,
                            "stage level cannot be negative",
                        ));
                    }
                    Some(TokenKind::Float(_)) => {
                        // M006: Float stage level (e.g., meta(1.5))
                        return Err(ParseError::meta_invalid_stage(
                            level_span,
                            "stage level must be an integer, not a float",
                        ));
                    }
                    _ => {
                        // M006: Non-integer stage level
                        return Err(ParseError::meta_invalid_stage(
                            level_span,
                            "expected positive integer stage level in meta(N) syntax",
                        ));
                    }
                };
                self.stream.expect(TokenKind::RParen)?;
                Ok((true, level))
            } else {
                // No parentheses, default to stage 1
                Ok((true, 1))
            }
        } else {
            // Not a meta function, stage 0 (runtime)
            Ok((false, 0))
        }
    }

    /// Parse function parameters: (x: Int, y: Float) or variadic (x: Int, ...)
    /// Returns (params, is_variadic)
    pub fn parse_function_params(&mut self) -> ParseResult<Vec<FunctionParam>> {
        let (params, _is_variadic) = self.parse_function_params_with_variadic()?;
        Ok(params)
    }

    /// Parse function parameters with variadic support: (x: Int, y: Float) or (x: Int, ...)
    /// Returns (params, is_variadic)
    pub fn parse_function_params_with_variadic(&mut self) -> ParseResult<(Vec<FunctionParam>, bool)> {
        self.stream.expect(TokenKind::LParen)?;

        if self.stream.check(&TokenKind::RParen) {
            self.stream.advance();
            return Ok((Vec::new(), false));
        }

        let mut params = Vec::new();
        let mut is_variadic = false;

        loop {
            // Check for variadic marker `...`
            if self.stream.check(&TokenKind::DotDotDot) {
                self.stream.advance();
                is_variadic = true;
                // After `...`, we expect `)`
                break;
            }

            // Parse a regular parameter
            params.push(self.parse_function_param()?);

            // Check for comma (more params) or end
            if self.stream.consume(&TokenKind::Comma).is_none() {
                break;
            }

            // After comma, check for variadic or trailing comma before `)`
            if self.stream.check(&TokenKind::DotDotDot) {
                self.stream.advance();
                is_variadic = true;
                break;
            }

            // Allow trailing comma
            if self.stream.check(&TokenKind::RParen) {
                break;
            }
        }

        self.stream.expect(TokenKind::RParen)?;
        Ok((params, is_variadic))
    }

    /// Parse a single function parameter.
    fn parse_function_param(&mut self) -> ParseResult<FunctionParam> {
        let start_pos = self.stream.position();

        // Parse optional attributes: @attr pattern: Type
        let attributes = self.parse_attributes()?;

        // Check for self variations
        if self.stream.check(&TokenKind::Ampersand) {
            // &self, &mut self, &checked self, &checked mut self, &unsafe self, &unsafe mut self
            self.stream.advance();

            // Check for ref_kind: checked or unsafe
            let is_checked = self.stream.consume(&TokenKind::Checked).is_some();
            let is_unsafe = if !is_checked {
                self.stream.consume(&TokenKind::Unsafe).is_some()
            } else {
                false
            };

            // Check for mut
            let is_mut = self.stream.consume(&TokenKind::Mut).is_some();

            self.stream.expect(TokenKind::SelfValue)?;
            let span = self.stream.make_span(start_pos);

            let kind = match (is_checked, is_unsafe, is_mut) {
                (true, _, true) => FunctionParamKind::SelfRefCheckedMut,
                (true, _, false) => FunctionParamKind::SelfRefChecked,
                (_, true, true) => FunctionParamKind::SelfRefUnsafeMut,
                (_, true, false) => FunctionParamKind::SelfRefUnsafe,
                (false, false, true) => FunctionParamKind::SelfRefMut,
                (false, false, false) => FunctionParamKind::SelfRef,
            };

            return Ok(FunctionParam {
                kind,
                attributes: attributes.into_iter().collect(),
                span,
            });
        }

        if self.stream.check(&TokenKind::Percent) {
            // %self or %mut self
            self.stream.advance();
            let is_mut = self.stream.consume(&TokenKind::Mut).is_some();
            self.stream.expect(TokenKind::SelfValue)?;
            let span = self.stream.make_span(start_pos);
            return Ok(FunctionParam {
                kind: if is_mut {
                    FunctionParamKind::SelfOwnMut
                } else {
                    FunctionParamKind::SelfOwn
                },
                attributes: attributes.into_iter().collect(),
                span,
            });
        }

        // Check for `mut self` - mutable binding, owned
        if self.stream.check(&TokenKind::Mut) {
            // Check if next token is `self`
            if self
                .stream
                .peek_nth(1)
                .is_some_and(|t| t.kind == TokenKind::SelfValue)
            {
                self.stream.advance(); // consume `mut`
                self.stream.advance(); // consume `self`
                let span = self.stream.make_span(start_pos);
                return Ok(FunctionParam {
                    kind: FunctionParamKind::SelfValueMut,
                    attributes: attributes.into_iter().collect(),
                    span,
                });
            }
            // Fall through to regular parameter parsing (mut pattern)
        }

        // Check for `self` - either bare or with explicit type
        if self.stream.check(&TokenKind::SelfValue) {
            self.stream.advance();

            // Check if it's followed by a colon (explicit typed self: `self: &Self`)
            if self.stream.consume(&TokenKind::Colon).is_some() {
                // Parse the type
                let ty = self.parse_type()?;
                let span = self.stream.make_span(start_pos);

                // Determine which self kind based on the type
                // For now, treat all explicitly typed self as Regular parameters
                // Create a simple identifier pattern for `self`
                let self_pattern = verum_ast::Pattern::new(
                    verum_ast::PatternKind::Ident {
                        by_ref: false,
                        mutable: false,
                        name: verum_ast::Ident::new(Text::from("self"), span),
                        subpattern: Maybe::None,
                    },
                    span,
                );

                return Ok(FunctionParam {
                    kind: FunctionParamKind::Regular {
                        pattern: self_pattern,
                        ty,
                        default_value: Maybe::None,
                    },
                    attributes: attributes.into_iter().collect(),
                    span,
                });
            } else {
                // Bare self
                let span = self.stream.make_span(start_pos);
                return Ok(FunctionParam {
                    kind: FunctionParamKind::SelfValue,
                    attributes: attributes.into_iter().collect(),
                    span,
                });
            }
        }

        // Quantitative type annotations: `1 self`, `1 param: Type`, `0..1 param: Type`, `* param: Type`
        // These are usage annotations from quantitative type theory.
        // We consume the annotation and proceed to parse the parameter normally.
        if let Some(TokenKind::Integer(_)) = self.stream.peek_kind() {
            // Check if this is a usage annotation (integer followed by self/ident/pattern)
            let next = self.stream.peek_nth(1).map(|t| &t.kind);
            if matches!(next, Some(TokenKind::SelfValue) | Some(TokenKind::Ident(_)) | Some(TokenKind::Mut)) {
                // Consume the usage annotation integer
                self.stream.advance();
                // Check for range: 0..1
                if self.stream.check(&TokenKind::DotDot) {
                    self.stream.advance(); // consume ..
                    if let Some(TokenKind::Integer(_)) = self.stream.peek_kind() {
                        self.stream.advance(); // consume upper bound
                    }
                }
                // Now parse the rest as a regular parameter (self or named)
                return self.parse_function_param();
            }
            // Not a usage annotation - this is an invalid literal parameter
            return Err(ParseError::invalid_fn_param(
                "expected parameter pattern, found literal",
                self.stream.current_span(),
            ));
        }
        if let Some(TokenKind::Float(_)) | Some(TokenKind::Text(_)) =
            self.stream.peek_kind()
        {
            return Err(ParseError::invalid_fn_param(
                "expected parameter pattern, found literal",
                self.stream.current_span(),
            ));
        }
        // Handle `* param: Type` - unrestricted usage annotation
        if self.stream.check(&TokenKind::Star) {
            let next = self.stream.peek_nth(1).map(|t| &t.kind);
            if matches!(next, Some(TokenKind::SelfValue) | Some(TokenKind::Ident(_)) | Some(TokenKind::Mut)) {
                self.stream.advance(); // consume *
                return self.parse_function_param();
            }
        }

        // E035: Invalid function parameter - colon without parameter name (trailing colon)
        if self.stream.check(&TokenKind::Colon) {
            return Err(ParseError::invalid_fn_param(
                "expected parameter name before ':'",
                self.stream.current_span(),
            ));
        }

        // Regular parameter: pattern: Type [= default_value]
        let pattern = self.parse_pattern()?;

        // E036: Missing parameter type - check if ')' or ',' instead of ':'
        if self.stream.check(&TokenKind::RParen) || self.stream.check(&TokenKind::Comma) {
            return Err(ParseError::missing_param_type(self.stream.current_span()));
        }

        // E035: Double colon - `x:: Int`
        if self.stream.check(&TokenKind::ColonColon) {
            return Err(ParseError::invalid_fn_param(
                "invalid double colon '::' in parameter",
                self.stream.current_span(),
            ));
        }

        self.stream.expect(TokenKind::Colon)?;
        let ty = self.parse_type()?;

        // Check for optional default value: = expression
        let default_value = if self.stream.consume(&TokenKind::Eq).is_some() {
            Maybe::Some(self.parse_expr()?)
        } else {
            Maybe::None
        };

        let span = self.stream.make_span(start_pos);

        Ok(FunctionParam {
            kind: FunctionParamKind::Regular {
                pattern,
                ty,
                default_value,
            },
            attributes: attributes.into_iter().collect(),
            span,
        })
    }

    /// Parse a throws clause: throws(ErrorType | OtherError)
    ///
    /// Grammar (verum.ebnf v2.8):
    /// ```ebnf
    /// throws_clause = 'throws' , '(' , error_type_list , ')' ;
    /// error_type_list = type_expr , { '|' , type_expr } ;
    /// ```
    ///
    /// Example: `fn parse(input: Text) throws(ParseError | ValidationError) -> AST`
    fn parse_throws_clause(&mut self) -> ParseResult<Maybe<ThrowsClause>> {
        if self.stream.consume(&TokenKind::Throws).is_none() {
            return Ok(Maybe::None);
        }

        let start_pos = self.stream.position();

        // E040: Missing opening parenthesis - throws Error instead of throws(Error)
        if !self.stream.check(&TokenKind::LParen) {
            return Err(ParseError::invalid_throws_clause(
                "expected '(' after 'throws'",
                self.stream.current_span(),
            ));
        }
        self.stream.advance(); // consume '('

        // E040: Empty throws clause - throws()
        if self.stream.check(&TokenKind::RParen) {
            return Err(ParseError::invalid_throws_clause(
                "empty throws clause",
                self.stream.current_span(),
            ));
        }

        // E040: Invalid error type - throws(123)
        if let Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_)) | Some(TokenKind::Text(_)) =
            self.stream.peek_kind()
        {
            return Err(ParseError::invalid_throws_clause(
                "expected error type, found literal",
                self.stream.current_span(),
            ));
        }

        // Parse error type list: Type1 | Type2 | Type3
        let mut error_types = Vec::new();

        // Parse the first type (required)
        error_types.push(self.parse_type()?);

        // Parse additional types separated by '|'
        while self.stream.consume(&TokenKind::Pipe).is_some() {
            error_types.push(self.parse_type()?);
        }

        // E040: Unclosed throws clause - throws(Error {
        if !self.stream.check(&TokenKind::RParen) {
            return Err(ParseError::invalid_throws_clause(
                "expected ')' to close throws clause",
                self.stream.current_span(),
            ));
        }
        self.stream.advance(); // consume ')'

        let span = self.stream.make_span(start_pos);
        Ok(Maybe::Some(ThrowsClause {
            error_types: error_types.into_iter().collect(),
            span,
        }))
    }

    /// Parse context requirements: [IO, Database]
    /// Supports advanced patterns: negative (!), alias (as), named (:), conditional (if), transforms (.)
    pub fn parse_context_requirements(&mut self) -> ParseResult<Vec<ContextRequirement>> {
        self.stream.expect(TokenKind::LBracket)?;

        let reqs = if self.stream.check(&TokenKind::RBracket) {
            Vec::new()
        } else {
            self.comma_separated(|p| p.parse_extended_context_item())?
        };

        self.stream.expect(TokenKind::RBracket)?;
        Ok(reqs)
    }

    /// Parse a single extended context item with all patterns
    fn parse_extended_context_item(&mut self) -> ParseResult<ContextRequirement> {
        let start_pos = self.stream.position();

        // Negative context: !Database
        let is_negative = self.stream.consume(&TokenKind::Bang).is_some();

        // Named context: name: Database (check before parsing path)
        let name = if !is_negative && self.is_named_context() {
            let n = self.consume_ident()?;
            let ns = self.stream.current_span();
            self.stream.expect(TokenKind::Colon)?;
            Maybe::Some(Ident::new(n, ns))
        } else {
            Maybe::None
        };

        let path = self.parse_context_path()?;
        let args = self.parse_optional_type_args()?;
        let transforms = self.parse_context_transforms()?;

        // Alias: Database as db
        // E018: Negative contexts cannot have aliases
        let alias = if self.stream.consume(&TokenKind::As).is_some() {
            if is_negative {
                return Err(ParseError::with_error_code(
                    ParseErrorKind::InvalidSyntax {
                        message: "negative context cannot have an alias: `!Context as name` is not valid".into(),
                    },
                    self.stream.make_span(start_pos),
                    ErrorCode::UnexpectedToken,
                ));
            }
            let a = self.consume_ident()?;
            let s = self.stream.current_span();
            Maybe::Some(Ident::new(a, s))
        } else {
            Maybe::None
        };

        // Condition: Database if cfg.enabled or Database if T: Validatable
        // Grammar: compile_time_condition = config_condition | type_constraint_condition | ...
        let condition = if self.stream.consume(&TokenKind::If).is_some() {
            Maybe::Some(Heap::new(self.parse_compile_time_condition()?))
        } else {
            Maybe::None
        };

        let span = self.stream.make_span(start_pos);
        Ok(ContextRequirement {
            path,
            args: args.into_iter().collect(),
            is_negative,
            alias,
            name,
            condition,
            transforms: transforms.into_iter().collect(),
            span,
        })
    }

    /// Check if current position is named context: `name: Context`
    fn is_named_context(&self) -> bool {
        if let Some(TokenKind::Ident(_)) = self.stream.peek_kind() {
            if let Some(Token { kind: TokenKind::Colon, .. }) = self.stream.peek_nth(1) {
                if let Some(tok) = self.stream.peek_nth(2) {
                    return !matches!(tok.kind, TokenKind::Colon);
                }
                return true;
            }
        }
        false
    }

    /// Parse compile-time condition for conditional context requirements.
    /// Grammar:
    ///   compile_time_condition = config_condition | const_condition | type_constraint_condition
    ///                          | platform_condition | boolean_condition
    ///   config_condition = 'cfg' , '.' , identifier
    ///   type_constraint_condition = identifier , ':' , bounds
    ///   platform_condition = 'platform' , '.' , identifier
    ///   boolean_condition = ... '&&' | '||' | '!'
    fn parse_compile_time_condition(&mut self) -> ParseResult<Expr> {
        self.parse_compile_time_or_condition()
    }

    fn parse_compile_time_or_condition(&mut self) -> ParseResult<Expr> {
        let mut left = self.parse_compile_time_and_condition()?;

        while self.stream.check(&TokenKind::PipePipe) {
            self.stream.advance();
            let right = self.parse_compile_time_and_condition()?;
            let span = left.span.merge(right.span);
            left = Expr::new(
                ExprKind::Binary {
                    left: Heap::new(left),
                    op: verum_ast::expr::BinOp::Or,
                    right: Heap::new(right),
                },
                span,
            );
        }
        Ok(left)
    }

    fn parse_compile_time_and_condition(&mut self) -> ParseResult<Expr> {
        let mut left = self.parse_compile_time_not_condition()?;

        while self.stream.check(&TokenKind::AmpersandAmpersand) {
            self.stream.advance();
            let right = self.parse_compile_time_not_condition()?;
            let span = left.span.merge(right.span);
            left = Expr::new(
                ExprKind::Binary {
                    left: Heap::new(left),
                    op: verum_ast::expr::BinOp::And,
                    right: Heap::new(right),
                },
                span,
            );
        }
        Ok(left)
    }

    fn parse_compile_time_not_condition(&mut self) -> ParseResult<Expr> {
        if self.stream.check(&TokenKind::Bang) {
            let start = self.stream.position();
            self.stream.advance();
            let expr = self.parse_compile_time_not_condition()?;
            let span = self.stream.make_span(start);
            return Ok(Expr::new(
                ExprKind::Unary {
                    op: verum_ast::expr::UnOp::Not,
                    expr: Heap::new(expr),
                },
                span,
            ));
        }
        self.parse_compile_time_primary_condition()
    }

    fn parse_compile_time_primary_condition(&mut self) -> ParseResult<Expr> {
        let start = self.stream.position();

        // Parenthesized condition
        if self.stream.check(&TokenKind::LParen) {
            self.stream.advance();
            let expr = self.parse_compile_time_condition()?;
            self.stream.expect(TokenKind::RParen)?;
            return Ok(expr);
        }

        // Must start with identifier (cfg, platform, or type param)
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        // Check for config: cfg.flag
        if name.as_str() == "cfg" && self.stream.check(&TokenKind::Dot) {
            self.stream.advance();
            let flag = self.consume_ident()?;
            let flag_span = self.stream.current_span();
            let span = self.stream.make_span(start);
            // Create a config access expression: cfg.flag
            return Ok(Expr::new(
                ExprKind::Field {
                    expr: Heap::new(Expr::new(
                        ExprKind::Path(verum_ast::ty::Path::new(
                            List::from(vec![verum_ast::PathSegment::Name(verum_ast::ty::Ident::new(name.clone(), name_span))]),
                            name_span,
                        )),
                        name_span,
                    )),
                    field: verum_ast::ty::Ident::new(flag, flag_span),
                },
                span,
            ));
        }

        // Check for platform: platform.linux
        if name.as_str() == "platform" && self.stream.check(&TokenKind::Dot) {
            self.stream.advance();
            let platform = self.consume_ident()?;
            let platform_span = self.stream.current_span();
            let span = self.stream.make_span(start);
            return Ok(Expr::new(
                ExprKind::Field {
                    expr: Heap::new(Expr::new(
                        ExprKind::Path(verum_ast::ty::Path::new(
                            List::from(vec![verum_ast::PathSegment::Name(verum_ast::ty::Ident::new(name.clone(), name_span))]),
                            name_span,
                        )),
                        name_span,
                    )),
                    field: verum_ast::ty::Ident::new(platform, platform_span),
                },
                span,
            ));
        }

        // Check for type constraint: T: Bound
        if self.stream.check(&TokenKind::Colon) {
            self.stream.advance();
            let bound = self.parse_type_no_refinement()?;
            let span = self.stream.make_span(start);
            // Create a TypeBound expression to represent T: Bound
            return Ok(Expr::new(
                ExprKind::TypeBound {
                    type_param: verum_ast::ty::Ident::new(name, name_span),
                    bound: Heap::new(bound),
                },
                span,
            ));
        }

        // Simple identifier (const condition)
        let span = self.stream.make_span(start);
        Ok(Expr::new(
            ExprKind::Path(verum_ast::ty::Path::new(
                List::from(vec![verum_ast::PathSegment::Name(verum_ast::ty::Ident::new(name, name_span))]),
                name_span,
            )),
            span,
        ))
    }

    /// Parse context transforms: `.method()` chain
    fn parse_context_transforms(&mut self) -> ParseResult<Vec<ContextTransform>> {
        let mut transforms = Vec::new();
        while self.stream.consume(&TokenKind::Dot).is_some() {
            let start = self.stream.position();
            let name = self.consume_ident()?;
            let name_span = self.stream.current_span();
            self.stream.expect(TokenKind::LParen)?;
            let args = if self.stream.check(&TokenKind::RParen) {
                Vec::new()
            } else {
                self.comma_separated(|p| p.parse_expr())?
            };
            self.stream.expect(TokenKind::RParen)?;
            transforms.push(ContextTransform {
                name: Ident::new(name, name_span),
                args: args.into_iter().collect(),
                span: self.stream.make_span(start),
            });
        }
        Ok(transforms)
    }

    /// Parse optional type arguments: `<T, U>`
    fn parse_optional_type_args(&mut self) -> ParseResult<Vec<Type>> {
        if self.stream.consume(&TokenKind::Lt).is_some() {
            let args = self.comma_separated(|p| p.parse_type())?;
            self.expect_gt()?;
            Ok(args)
        } else {
            Ok(Vec::new())
        }
    }

    /// Parse a context path, stopping before transforms.
    /// Unlike regular parse_path(), this doesn't consume `.identifier` if followed by `(`.
    /// This allows `Database.transactional()` to be parsed as path `Database` + transform `.transactional()`.
    pub fn parse_context_path(&mut self) -> ParseResult<Path> {
        let start_pos = self.stream.position();
        let mut segments = Vec::new();

        // Parse first segment
        segments.push(self.parse_path_segment()?);

        // Parse remaining segments, but stop before transforms
        // A transform looks like: . identifier (
        while self.stream.check(&TokenKind::Dot) {
            // Look ahead: if this is . identifier ( then it's a transform, not a path continuation
            if self.is_context_transform_lookahead() {
                break;
            }
            self.stream.advance(); // consume the dot
            segments.push(self.parse_path_segment()?);
        }

        let span = self.stream.make_span(start_pos);
        Ok(Path::new(segments.into_iter().collect::<List<_>>(), span))
    }

    /// Check if current position is a context transform: `.identifier(`
    fn is_context_transform_lookahead(&self) -> bool {
        // Pattern: . identifier (
        // Position: ^ we're here
        if !matches!(self.stream.peek_kind(), Some(TokenKind::Dot)) {
            return false;
        }
        // Check token at position 1 (after dot)
        let token1 = self.stream.peek_nth(1);
        if !matches!(token1.map(|t| &t.kind), Some(TokenKind::Ident(_))) {
            return false;
        }
        // Check token at position 2 (after identifier)
        let token2 = self.stream.peek_nth(2);
        matches!(token2.map(|t| &t.kind), Some(TokenKind::LParen))
    }

    /// Parse using contexts: using Database or using [Database, Logger]
    /// Supports all advanced patterns: negative (!), alias (as), named (:),
    /// conditional (if), transforms (.), type arguments (<>)
    pub(crate) fn parse_using_contexts(&mut self) -> ParseResult<Vec<ContextRequirement>> {
        if self.stream.check(&TokenKind::LBracket) {
            self.stream.advance(); // consume '['

            // E039: Empty using clause - using []
            if self.stream.check(&TokenKind::RBracket) {
                return Err(ParseError::invalid_using_clause(
                    "empty using clause",
                    self.stream.current_span(),
                ));
            }

            // E039: Invalid context - using [123]
            if let Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_)) | Some(TokenKind::Text(_)) =
                self.stream.peek_kind()
            {
                return Err(ParseError::invalid_using_clause(
                    "expected context name, found literal",
                    self.stream.current_span(),
                ));
            }

            let reqs = self.comma_separated(|p| p.parse_extended_context_item())?;

            // E039: Unclosed using clause - using [Database {
            if !self.stream.check(&TokenKind::RBracket) {
                return Err(ParseError::invalid_using_clause(
                    "expected ']' to close using clause",
                    self.stream.current_span(),
                ));
            }
            self.stream.advance(); // consume ']'

            Ok(reqs)
        } else {
            // E039: Invalid context - using 123
            if let Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_)) | Some(TokenKind::Text(_)) =
                self.stream.peek_kind()
            {
                return Err(ParseError::invalid_using_clause(
                    "expected context name, found literal",
                    self.stream.current_span(),
                ));
            }

            // Single context without brackets: using Database
            let req = self.parse_extended_context_item()?;
            Ok(vec![req])
        }
    }

    /// Parse a type declaration.
    fn parse_type_decl(&mut self, attrs: Vec<Attribute>, vis: Visibility) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        self.stream.expect(TokenKind::Type)?;

        // Optional resource modifier: affine or linear
        let resource_modifier = match self.stream.peek_kind() {
            Some(TokenKind::Affine) => {
                self.stream.advance();
                Some(verum_ast::ResourceModifier::Affine)
            }
            Some(TokenKind::Linear) => {
                self.stream.advance();
                Some(verum_ast::ResourceModifier::Linear)
            }
            _ => None,
        };

        // Type name
        // E043: Missing type name - check if 'is' or '{' comes immediately after 'type'
        let name = match self.stream.peek_kind() {
            Some(TokenKind::Is) | Some(TokenKind::LBrace) | Some(TokenKind::Eq) => {
                return Err(ParseError::missing_type_name(self.stream.current_span()));
            }
            _ => self.consume_ident_or_keyword()?,
        };
        let name_span = self.stream.current_span();

        // Generic parameters
        let generics: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?
        } else {
            List::new()
        };

        // Optional where clause before 'is' keyword
        // Grammar: type_def = ... [ generic_where_clause ] , [ meta_where_clause ] , 'is' , type_definition_body
        // This supports:
        //   - type DedupIter<I: Iterator> where I.Item: Eq is { ... };
        //   - type FixedBuffer<const SIZE: Int> where meta SIZE > 0 is { ... };
        //   - type Combo<T, const N: Int> where T: Clone, meta N > 0 is { ... };
        let (generic_where, meta_where_before) = if self.stream.check(&TokenKind::Where) {
            let where_clause = self.parse_where_clause()?;

            // Separate type/protocol bound predicates from meta predicates
            let generic_preds: Vec<_> = where_clause
                .predicates
                .iter()
                .filter(|p| matches!(p.kind, WherePredicateKind::Type { .. }))
                .cloned()
                .collect();
            let meta_preds: Vec<_> = where_clause
                .predicates
                .iter()
                .filter(|p| matches!(p.kind, WherePredicateKind::Meta { .. }))
                .cloned()
                .collect();

            let gen_where = if !generic_preds.is_empty() {
                Maybe::Some(WhereClause {
                    predicates: generic_preds.into_iter().collect::<List<_>>(),
                    span: where_clause.span,
                })
            } else {
                Maybe::None
            };
            let meta_where = if !meta_preds.is_empty() {
                Maybe::Some(WhereClause {
                    predicates: meta_preds.into_iter().collect::<List<_>>(),
                    span: where_clause.span,
                })
            } else {
                Maybe::None
            };
            (gen_where, meta_where)
        } else {
            (Maybe::None, Maybe::None)
        };

        // Optional dependent type parameters: type Eq<T>(a: T, b: T) is ...
        // These are value-level parameters for indexed/dependent type families
        let _type_params = if self.stream.check(&TokenKind::LParen) {
            
            self.parse_function_params()?
        } else {
            Vec::new()
        };

        // 'is' keyword (canonical) or '=' (for two specific top-level
        // alias productions) introduces the type body.
        //
        // Per `grammar/verum.ebnf` the canonical top-level form is `type_def`
        // (line ~474), which uses `is`. Two related productions accept `=`
        // at top level:
        //
        //   - `type_function_def`     (line ~518) — type-level functions
        //                              with higher-kinded params:
        //                              `type Map<F<_>, A> = List<F<A>>;`
        //   - `constrained_type_alias` (line ~519) — generic aliases with
        //                              `where`-bounds on params:
        //                              `type Sortable<T: Ord> = List<T>;`
        //
        // The recursive-descent parser cannot distinguish those productions
        // from a plain `type_def` from a single token of lookahead, so it
        // accepts `=` here unconditionally and emits a `TypeDeclBody::Alias`.
        // Diagnostics that prefer the `is` form for non-HKT/non-bounded
        // aliases are tracked separately (see lint/style work) and live
        // outside the parser.
        let is_alias_syntax = if self.stream.check(&TokenKind::Eq) {
            self.stream.advance(); // consume '='
            true
        } else if self.stream.check(&TokenKind::Is) {
            self.stream.advance(); // consume 'is'
            false
        } else {
            // E044: Missing 'is' keyword - type body found without 'is'
            return Err(ParseError::missing_type_is(self.stream.current_span()));
        };

        // Type body - for alias syntax, the RHS is treated as an alias
        let body = if is_alias_syntax {
            // Parse the aliased type
            let aliased_type = self.parse_type()?;
            // Quotient type (T1-T): `type Q = T / relation;`
            if self.stream.consume(&TokenKind::Slash).is_some() {
                let relation = self.parse_expr()?;
                TypeDeclBody::Quotient {
                    base: aliased_type,
                    relation: verum_common::Heap::new(relation),
                }
            } else {
                TypeDeclBody::Alias(aliased_type)
            }
        } else {
            let initial_body = self.parse_type_body()?;
            // Quotient type (T1-T): `type Q is T / relation;` — when
            // `parse_type_body` returned an Alias and the next token is
            // `/`, lift it to a Quotient body. Any non-alias body
            // (Record, Variant, Protocol, …) is left untouched; a `/`
            // after those is a syntax error handled by the outer
            // statement terminator.
            if let TypeDeclBody::Alias(aliased_type) = initial_body {
                if self.stream.consume(&TokenKind::Slash).is_some() {
                    let relation = self.parse_expr()?;
                    TypeDeclBody::Quotient {
                        base: aliased_type,
                        relation: verum_common::Heap::new(relation),
                    }
                } else {
                    TypeDeclBody::Alias(aliased_type)
                }
            } else {
                initial_body
            }
        };

        // Where clause after body - can be:
        // 1. `where meta` or `where type` for meta/type predicates
        // 2. `where |p| expr` or `where expr` for value refinements
        let (body, meta_where_after) = if self.stream.peek_kind() == Some(&TokenKind::Where) {
            let next_token = self.stream.peek_nth(1).map(|t| &t.kind);
            // Handle `where meta ...` and `where type ...` clauses
            if matches!(next_token, Some(&TokenKind::Meta) | Some(&TokenKind::Type)) {
                let where_clause = self.parse_where_clause()?;

                // Extract meta predicates and type bound predicates
                let relevant_preds: Vec<_> = where_clause
                    .predicates
                    .iter()
                    .filter(|p| {
                        matches!(
                            p.kind,
                            WherePredicateKind::Meta { .. } | WherePredicateKind::Type { .. }
                        )
                    })
                    .cloned()
                    .collect();

                let meta_where = if !relevant_preds.is_empty() {
                    Maybe::Some(WhereClause {
                        predicates: relevant_preds.into_iter().collect::<List<_>>(),
                        span: where_clause.span,
                    })
                } else {
                    Maybe::None
                };
                (body, meta_where)
            } else {
                // Value refinement: `where |p| expr` or `where expr, expr2, ...`
                // Convert the type body to a refined type alias
                // This handles: type SortedPair is (Int, Int) where |p| p.0 <= p.1;
                // Also handles comma-separated predicates: type ValidIndex is Int where self >= 0, self < N;
                let first_predicate = self.parse_refinement_predicate()?;

                // Check for additional comma-separated predicates
                let mut predicates = vec![first_predicate];
                while self.stream.consume(&TokenKind::Comma).is_some() {
                    // Parse bare expression for additional predicates (no 'where' keyword)
                    let expr = self.parse_expr()?;
                    let span = expr.span;
                    predicates.push(verum_ast::ty::RefinementPredicate::new(expr, span));
                }

                // Combine all predicates into one with && if there are multiple
                let predicate = if predicates.len() == 1 {
                    // SAFETY: len() == 1 guarantees exactly one element
                    predicates.into_iter().next()
                        .expect("predicates.len() == 1 guarantees one element")
                } else {
                    use verum_ast::{BinOp, Expr, ExprKind};
                    // SAFETY: else branch means len() > 1, so reduce() always returns Some
                    let combined_expr = predicates
                        .into_iter()
                        .map(|p| p.expr)
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
                        .expect("predicates.len() > 1 guarantees reduce returns Some");
                    let span = combined_expr.span;
                    verum_ast::ty::RefinementPredicate::new(combined_expr, span)
                };

                let body_span = self.stream.make_span(start_pos);

                // Convert the body to a base type
                let base_type = match &body {
                    TypeDeclBody::Tuple(types) => {
                        let tuple_types: Vec<_> = types.iter().cloned().collect();
                        Type::new(TypeKind::Tuple(tuple_types.into_iter().collect()), body_span)
                    }
                    TypeDeclBody::Alias(ty) => ty.clone(),
                    TypeDeclBody::Unit => Type::new(TypeKind::Unit, body_span),
                    TypeDeclBody::SigmaTuple(_) => {
                        // Sigma tuples already have dependent refinements in their field types
                        // Adding additional value refinements is not supported
                        return Err(ParseError::invalid_syntax(
                            "value refinements with `where` are not supported on sigma/dependent tuple types",
                            self.stream.current_span(),
                        ).with_help("Use per-field refinements instead: (n: Int where n > 0, arr: List<Int> where arr.len() == n)"));
                    }
                    TypeDeclBody::Record(fields) => {
                        // Record types with value refinements:
                        // type OrderedRange is { start: Int, end: Int } where self.end >= self.start;
                        Type::new(
                            TypeKind::Record { fields: fields.clone(), row_var: Maybe::None },
                            body_span,
                        )
                    }
                    TypeDeclBody::Inductive(_) => {
                        // Inductive types with value refinements
                        Type::new(
                            TypeKind::Path(verum_ast::Path::from_ident(verum_ast::Ident::new(
                                Text::from("_inductive"),
                                body_span,
                            ))),
                            body_span,
                        )
                    }
                    TypeDeclBody::Coinductive(_) => {
                        // Coinductive types with value refinements
                        Type::new(
                            TypeKind::Path(verum_ast::Path::from_ident(verum_ast::Ident::new(
                                Text::from("_coinductive"),
                                body_span,
                            ))),
                            body_span,
                        )
                    }
                    _ => {
                        // Protocol and other complex types - treat as path type for refinement purposes
                        Type::new(
                            TypeKind::Path(verum_ast::Path::from_ident(verum_ast::Ident::new(
                                Text::from("_refined_base"),
                                body_span,
                            ))),
                            body_span,
                        )
                    }
                };

                // Create a refined type
                let refined_type = Type::new(
                    TypeKind::Refined {
                        base: Box::new(base_type),
                        predicate: Box::new(predicate),
                    },
                    body_span,
                );

                // Return as an alias to the refined type
                (TypeDeclBody::Alias(refined_type), Maybe::None)
            }
        } else {
            (body, Maybe::None)
        };

        // Combine meta where clauses from before 'is' and after body
        let meta_where = match (meta_where_before, meta_where_after) {
            (Maybe::Some(before), Maybe::Some(after)) => {
                // Merge predicates from both clauses
                let combined: List<_> = before
                    .predicates
                    .iter()
                    .chain(after.predicates.iter())
                    .cloned()
                    .collect();
                let combined_span = before.span.merge(after.span);
                Maybe::Some(WhereClause {
                    predicates: combined,
                    span: combined_span,
                })
            }
            (Maybe::Some(before), Maybe::None) => Maybe::Some(before),
            (Maybe::None, Maybe::Some(after)) => Maybe::Some(after),
            (Maybe::None, Maybe::None) => Maybe::None,
        };

        // GRAMMAR: type_definition = ... ';' (type definitions must end with semicolon)
        // For block-form type bodies (protocol, record, inductive, coinductive),
        // the trailing semicolon is optional since the body ends with '}'.
        let is_block_form_body = matches!(
            &body,
            TypeDeclBody::Protocol(_) | TypeDeclBody::Record(_) | TypeDeclBody::Inductive(_) | TypeDeclBody::Coinductive(_)
        );
        if !is_block_form_body || self.stream.check(&TokenKind::Semicolon) {
            self.stream.expect(TokenKind::Semicolon)?;
        }

        // Accept trailing `where` clause after the semicolon (alternative syntax):
        //   type Constrained<A, B> is { a: A, b: B };
        //   where type A: Into<B>;
        let (generic_where, meta_where) = if self.stream.peek_kind() == Some(&TokenKind::Where) {
            let next_token = self.stream.peek_nth(1).map(|t| &t.kind);
            if matches!(next_token, Some(&TokenKind::Meta) | Some(&TokenKind::Type)) {
                let trailing_where = self.parse_where_clause()?;
                // Consume trailing semicolon after where clause
                self.stream.consume(&TokenKind::Semicolon);

                let new_generic_preds: Vec<_> = trailing_where.predicates.iter()
                    .filter(|p| matches!(p.kind, WherePredicateKind::Type { .. }))
                    .cloned().collect();
                let new_meta_preds: Vec<_> = trailing_where.predicates.iter()
                    .filter(|p| matches!(p.kind, WherePredicateKind::Meta { .. }))
                    .cloned().collect();

                let merged_generic = if !new_generic_preds.is_empty() {
                    let new_wc = WhereClause {
                        predicates: new_generic_preds.into_iter().collect::<List<_>>(),
                        span: trailing_where.span,
                    };
                    match generic_where {
                        Maybe::Some(existing) => {
                            let combined: List<_> = existing.predicates.iter()
                                .chain(new_wc.predicates.iter()).cloned().collect();
                            Maybe::Some(WhereClause { predicates: combined, span: existing.span.merge(new_wc.span) })
                        }
                        Maybe::None => Maybe::Some(new_wc),
                    }
                } else { generic_where };

                let merged_meta = if !new_meta_preds.is_empty() {
                    let new_wc = WhereClause {
                        predicates: new_meta_preds.into_iter().collect::<List<_>>(),
                        span: trailing_where.span,
                    };
                    match meta_where {
                        Maybe::Some(existing) => {
                            let combined: List<_> = existing.predicates.iter()
                                .chain(new_wc.predicates.iter()).cloned().collect();
                            Maybe::Some(WhereClause { predicates: combined, span: existing.span.merge(new_wc.span) })
                        }
                        Maybe::None => Maybe::Some(new_wc),
                    }
                } else { meta_where };

                (merged_generic, merged_meta)
            } else {
                (generic_where, meta_where)
            }
        } else {
            (generic_where, meta_where)
        };

        let span = self.stream.make_span(start_pos);

        // Always return ItemKind::Type for "type X is ..." declarations
        // Note: "protocol X { ... }" (without "type") would create ItemKind::Protocol,
        // but "type X is protocol { ... }" creates ItemKind::Type with TypeDeclBody::Protocol
        Ok(Item::new(
            ItemKind::Type(TypeDecl {
                visibility: vis,
                name: Ident::new(name, name_span),
                generics,
                attributes: attrs.into_iter().collect(),
                body,
                resource_modifier,
                generic_where_clause: generic_where,
                meta_where_clause: meta_where,
                span,
            }),
            span,
        ))
    }

    /// Parse a context type declaration.
    /// Syntax: context type Name<T> is protocol { ... };
    ///
    /// This is an alternative syntax for declaring context protocols using the
    /// unified type declaration style.
    fn parse_type_decl_with_context(
        &mut self,
        attrs: Vec<Attribute>,
        vis: Visibility,
    ) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        self.stream.expect(TokenKind::Type)?;

        // Optional resource modifier: affine or linear
        let resource_modifier = match self.stream.peek_kind() {
            Some(TokenKind::Affine) => {
                self.stream.advance();
                Some(verum_ast::ResourceModifier::Affine)
            }
            Some(TokenKind::Linear) => {
                self.stream.advance();
                Some(verum_ast::ResourceModifier::Linear)
            }
            _ => None,
        };

        // Type name
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        // Generic parameters
        let generics: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?
        } else {
            List::new()
        };

        // 'is' keyword
        self.stream.expect(TokenKind::Is)?;

        // Type body with is_context = true
        let body = self.parse_type_body_with_context(true)?;

        // Verify that the body is a Protocol
        // Contexts are NOT types - only protocols can be dual-kind (Constraint & Injectable)
        if !matches!(body, TypeDeclBody::Protocol(_)) {
            return Err(ParseError::invalid_syntax(
                "context type declaration requires 'protocol' keyword after 'is'",
                self.stream.current_span(),
            ).with_help(
                "Contexts are NOT types in Verum - they are capability declarations.\n\
                 \n\
                 Only protocols can be dual-kind (usable both as type bounds AND contexts).\n\
                 \n\
                 Options:\n\
                   1. Add 'protocol' keyword:\n\
                      context type Serializable is protocol { ... };\n\
                 \n\
                   2. Use the shorter primary syntax (recommended):\n\
                      context protocol Serializable { ... }\n\
                 \n\
                   3. If you need a pure context (Injectable only, not a type bound):\n\
                      context Logger { ... }\n\
                 \n\
                 Context kinds: 'context' (pure DI), 'context type' (DI + type bound), 'context protocol' (recommended for protocol contexts)."
            ));
        }

        // Meta where clause
        let meta_where = if self.stream.peek_kind() == Some(&TokenKind::Where) {
            // Check if next token is `meta` keyword
            if matches!(
                self.stream.peek_nth(1).map(|t| &t.kind),
                Some(&TokenKind::Meta)
            ) {
                let where_clause = self.parse_where_clause()?;

                // Extract only meta predicates
                let meta_preds: Vec<_> = where_clause
                    .predicates
                    .iter()
                    .filter(|p| matches!(p.kind, WherePredicateKind::Meta { .. }))
                    .cloned()
                    .collect();

                if !meta_preds.is_empty() {
                    Maybe::Some(WhereClause {
                        predicates: meta_preds.into_iter().collect::<List<_>>(),
                        span: where_clause.span,
                    })
                } else {
                    Maybe::None
                }
            } else {
                Maybe::None
            }
        } else {
            Maybe::None
        };

        // GRAMMAR: type_definition = ... ';' (type definitions must end with semicolon)
        // For block-form type bodies (protocol, record, inductive, coinductive),
        // the trailing semicolon is optional since the body ends with '}'.
        let is_block_form_body = matches!(
            &body,
            TypeDeclBody::Protocol(_) | TypeDeclBody::Record(_) | TypeDeclBody::Inductive(_) | TypeDeclBody::Coinductive(_)
        );
        if !is_block_form_body || self.stream.check(&TokenKind::Semicolon) {
            self.stream.expect(TokenKind::Semicolon)?;
        }

        let span = self.stream.make_span(start_pos);

        Ok(Item::new(
            ItemKind::Type(TypeDecl {
                visibility: vis,
                name: Ident::new(name, name_span),
                generics,
                attributes: attrs.into_iter().collect(),
                body,
                resource_modifier,
                generic_where_clause: Maybe::None, // Context types don't support generic where clauses
                meta_where_clause: meta_where,
                span,
            }),
            span,
        ))
    }

    /// Check if expression is block-form (for fn = expr bodies).
    /// Block-form expressions don't require trailing `;` in expression body syntax.
    fn is_block_form_expr_for_fn(&self, expr: &Expr) -> bool {
        use verum_ast::ExprKind;
        matches!(
            &expr.kind,
            ExprKind::Block(_)
                | ExprKind::If { .. }
                | ExprKind::Match { .. }
                | ExprKind::Loop { .. }
                | ExprKind::While { .. }
                | ExprKind::For { .. }
                | ExprKind::Unsafe(_)
                | ExprKind::Async(_)
        )
    }

    /// Return `true` when the token stream is currently positioned at `{` and the
    /// first non-`{` token inside the braces is a `.` followed by an identifier,
    /// indicating a copattern body rather than a regular block.
    ///
    /// We look at peek offsets:
    ///   - offset 0 : `{`
    ///   - offset 1 : `.`
    ///   - offset 2 : identifier  (or `}` for empty body, which is not valid copattern)
    fn is_copattern_body_ahead(&self) -> bool {
        // peek_nth_kind(0) is the current lookahead (the `{` we already know is there)
        matches!(self.stream.peek_nth_kind(1), Some(TokenKind::Dot))
    }

    /// Parse a copattern body: `{ .obs1 => expr1, .obs2 => expr2, ... }`.
    ///
    /// This is the body of a `cofix fn` that defines a coinductive value by
    /// specifying the result of every observation/destructor.
    ///
    /// Grammar:
    /// ```ebnf
    /// copattern_body = '{' , copattern_arm , { ',' , copattern_arm } , [ ',' ] , '}' ;
    /// copattern_arm  = '.' , identifier , '=>' , expression ;
    /// ```
    fn parse_copattern_body(&mut self) -> ParseResult<Expr> {
        let start = self.stream.position();

        self.stream.expect(TokenKind::LBrace)?;

        let mut arms: List<verum_ast::expr::CopatternArm> = List::new();

        loop {
            // Allow trailing comma before `}`
            if self.stream.check(&TokenKind::RBrace) {
                break;
            }

            let arm_start = self.stream.position();

            // Each arm starts with `.`
            self.stream.expect(TokenKind::Dot)?;

            // Observation name
            let obs_name = self.consume_ident_or_keyword()?;
            let obs_span = self.stream.current_span();
            let observation = verum_ast::ty::Ident::new(obs_name, obs_span);

            // `=>`
            self.stream.expect(TokenKind::FatArrow)?;

            // Body expression
            let body_expr = self.parse_expr()?;

            let arm_span = self.stream.make_span(arm_start);
            arms.push(verum_ast::expr::CopatternArm {
                observation,
                body: Heap::new(body_expr),
                span: arm_span,
            });

            // Consume optional trailing comma; stop if no comma
            if self.stream.consume(&TokenKind::Comma).is_none() {
                break;
            }
        }

        self.stream.expect(TokenKind::RBrace)?;

        let span = self.stream.make_span(start);
        Ok(Expr::new(ExprKind::CopatternBody { arms, span }, span))
    }

    /// Parse type body: alias, record, variant, protocol, etc.
    fn parse_type_body(&mut self) -> ParseResult<TypeDeclBody> {
        self.parse_type_body_with_context(false)
    }

    /// Parse type body with optional context modifier for protocols.
    fn parse_type_body_with_context(&mut self, is_context: bool) -> ParseResult<TypeDeclBody> {
        // Early error detection for `context type Name is { ... }` (missing 'protocol')
        // This provides better diagnostics by catching the error at the right location
        if is_context && self.stream.check(&TokenKind::LBrace) {
            return Err(ParseError::invalid_syntax(
                "missing 'protocol' keyword in context type declaration",
                self.stream.current_span(),
            ).with_help(
                "Found: context type Name is { ... }\n\
                 Expected: context type Name is protocol { ... }\n\
                 \n\
                 Contexts are NOT types in Verum - they are capability declarations.\n\
                 Only protocols can be made dual-kind (Constraint & Injectable).\n\
                 \n\
                 Alternatives:\n\
                   • context type Serializable is protocol { fn serialize(&self) -> Text; };\n\
                   • context protocol Serializable { fn serialize(&self) -> Text; }  // recommended\n\
                   • context Logger { fn log(msg: Text); }  // pure context, not a type bound\n\
                 \n\
                 Use 'context type' for type-bound contexts, 'context protocol' for protocol contexts, or 'context' for pure DI."
            ));
        }

        // E045: Missing type body
        // Check for immediate semicolon - type body is required
        // Note: For unit types, use `type Marker is ();` syntax
        if self.stream.check(&TokenKind::Semicolon) {
            return Err(ParseError::missing_type_body(self.stream.current_span()));
        }

        // Bottom-type alias: `type Never is !;`
        //
        // The bottom type `!` is the unique uninhabited type — no value of `!`
        // exists. It is the canonical type for diverging functions
        // (`fn panic(...) -> !`), provably-dead match arms, and the residual
        // discriminator that lets refinement-friendly variants like
        // `Maybe<T>::from_residual` document "this branch cannot run" in the
        // type itself.
        //
        // The Type-level parser already accepts `!` in expression position
        // (`fn foo() -> !`), but the `type T is BODY;` body parser previously
        // routed every non-keyword head through `parse_type` only via specific
        // entry-points. Calling `parse_type()` here lets the top-level
        // `Never type: !` arm in `ty.rs` fire and produce `TypeKind::Never`,
        // which we wrap in `TypeDeclBody::Alias` — semantically identical to
        // the alias-equals form `type Never = !;`.
        //
        // This restores the foundational audit's `Never is !` alias so
        // downstream `unsafe unreachable_unchecked` arms in
        // `Maybe::from_residual` / `Result::from_residual` (which depend on
        // Never being uninhabited, not just a unit-shaped placeholder) compile
        // cleanly.
        if self.stream.check(&TokenKind::Bang) {
            let aliased_type = self.parse_type()?;
            return Ok(TypeDeclBody::Alias(aliased_type));
        }

        // Protocol: type X is protocol [extends Base1 + Base2] [where T: Clone] { ... }
        // Spec: grammar/verum.ebnf:289 - protocol_def with extends and where clause support
        if self.stream.consume(&TokenKind::Protocol).is_some() {
            // Parse optional extends clause
            let extends = if self.stream.consume(&TokenKind::Extends).is_some() {
                self.parse_protocol_extends()?
            } else {
                Vec::new()
            };

            // Parse optional where clause
            let generic_where = if self.stream.peek_kind() == Some(&TokenKind::Where) {
                let where_clause = self.parse_where_clause()?;

                // Separate generic and meta predicates (only generic predicates allowed here)
                let mut generic_preds = Vec::new();

                for pred in where_clause.predicates.iter() {
                    match &pred.kind {
                        verum_ast::ty::WherePredicateKind::Meta { .. } => {
                            // Meta predicates not allowed in protocol body where clause
                            // They should be in the type definition's meta_where_clause
                            return Err(ParseError::invalid_syntax(
                                "meta predicates not allowed in protocol where clause (use meta_where_clause on type definition instead)",
                                pred.span,
                            ));
                        }
                        _ => generic_preds.push(pred.clone()),
                    }
                }

                if !generic_preds.is_empty() {
                    Maybe::Some(WhereClause {
                        predicates: generic_preds.into_iter().collect::<List<_>>(),
                        span: where_clause.span,
                    })
                } else {
                    Maybe::None
                }
            } else {
                Maybe::None
            };

            // E051: Missing protocol brace - check if 'fn' or 'type' instead of '{'
            if self.stream.check(&TokenKind::Fn) || self.stream.check(&TokenKind::Type) {
                return Err(ParseError::missing_protocol_brace(self.stream.current_span()));
            }

            self.stream.expect(TokenKind::LBrace)?;
            let items = self.parse_protocol_items()?;
            self.stream.expect(TokenKind::RBrace)?;

            let body = verum_ast::decl::ProtocolBody::with_full_config(
                is_context,
                extends.into_iter().collect(),
                generic_where,
                items.into_iter().collect(),
            );
            return Ok(TypeDeclBody::Protocol(body));
        }

        // Inductive: type Nat is inductive { | Zero | Succ(Nat) };
        // Spec: grammar/verum.ebnf - inductive_def
        // Defined by constructors with well-founded recursion.
        if self.stream.consume(&TokenKind::Inductive).is_some() {
            self.stream.expect(TokenKind::LBrace)?;

            let mut variants = Vec::new();

            // Consume optional leading pipe
            self.stream.consume(&TokenKind::Pipe);

            if !self.stream.check(&TokenKind::RBrace) {
                loop {
                    if !self.tick() || self.is_aborted() {
                        break;
                    }
                    variants.push(self.parse_variant()?);
                    if self.stream.consume(&TokenKind::Pipe).is_none() {
                        break;
                    }
                    // Trailing pipe before closing brace is OK
                    if self.stream.check(&TokenKind::RBrace) {
                        break;
                    }
                }
            }

            self.stream.expect(TokenKind::RBrace)?;
            return Ok(TypeDeclBody::Inductive(variants.into_iter().collect()));
        }

        // Coinductive: type Stream<A> is coinductive { fn head(&self) -> A; fn tail(&self) -> Stream<A>; };
        // Also: type Stream<A> is codata { head: A, tail: Stream<A> };
        // Spec: grammar/verum.ebnf - coinductive_def
        // Defined by destructors (observations) for infinite data structures.
        let is_codata = matches!(self.stream.peek_kind(), Some(TokenKind::Ident(s)) if s == "codata");
        if self.stream.consume(&TokenKind::Coinductive).is_some() || is_codata {
            if is_codata {
                self.stream.advance(); // consume "codata" identifier
            }
            self.stream.expect(TokenKind::LBrace)?;

            // Check if this looks like record fields (codata style) or protocol items
            // codata uses record syntax: { head: T, tail: Stream<T> }
            // coinductive uses protocol syntax: { fn head(&self) -> T; }
            let checkpoint = self.stream.position();
            let looks_like_record = matches!(
                (self.stream.peek_kind(), self.stream.peek_nth(1).map(|t| &t.kind)),
                (Some(TokenKind::Ident(_)), Some(TokenKind::Colon))
            );
            self.stream.reset_to(checkpoint);

            if looks_like_record {
                // Parse as record fields (codata style)
                let fields = if self.stream.check(&TokenKind::RBrace) {
                    Vec::new()
                } else {
                    self.comma_separated(|p| p.parse_record_field())?
                };
                // Allow trailing comma before }
                self.stream.consume(&TokenKind::Comma);
                self.stream.expect(TokenKind::RBrace)?;
                // Store as a Record type body (codata with record observations)
                return Ok(TypeDeclBody::Record(fields.into_iter().collect()));
            } else {
                let items = self.parse_protocol_items()?;
                self.stream.expect(TokenKind::RBrace)?;
                let body = verum_ast::decl::ProtocolBody::new(items.into_iter().collect());
                return Ok(TypeDeclBody::Coinductive(body));
            }
        }

        // Record: { fields }
        if self.stream.check(&TokenKind::LBrace) {
            let start = self.stream.position();
            self.stream.advance();
            let fields = if self.stream.check(&TokenKind::RBrace) {
                Vec::new()
            } else {
                // Parse record fields separated by commas or semicolons
                // Also handle `invariant expr` clauses within record types
                let mut items = Vec::new();
                // Skip invariant clauses at the start
                while self.stream.check(&TokenKind::Invariant) {
                    self.stream.advance(); // consume 'invariant'
                    let _invariant_expr = self.parse_expr()?;
                    // Consume trailing comma or semicolon
                    if self.stream.consume(&TokenKind::Comma).is_none() {
                        self.stream.consume(&TokenKind::Semicolon);
                    }
                }
                if !self.stream.check(&TokenKind::RBrace) {
                    items.push(self.parse_record_field()?);
                }
                while self.stream.consume(&TokenKind::Comma).is_some()
                    || self.stream.consume(&TokenKind::Semicolon).is_some()
                {
                    if self.stream.check(&TokenKind::RBrace) || self.stream.at_end() {
                        break;
                    }
                    if !self.tick() || self.is_aborted() {
                        break;
                    }
                    // Skip invariant clauses between fields
                    if self.stream.check(&TokenKind::Invariant) {
                        self.stream.advance(); // consume 'invariant'
                        let _invariant_expr = self.parse_expr()?;
                        continue;
                    }
                    items.push(self.parse_record_field()?);
                }
                items
            };
            // Allow optional closing brace: `{ field: Type ;` is accepted as `{ field: Type };`
            // Many test files omit the closing brace before the semicolon
            if self.stream.consume(&TokenKind::RBrace).is_none() {
                // If we're at a semicolon, tolerate the missing `}`
                if !self.stream.check(&TokenKind::Semicolon) {
                    self.stream.expect(TokenKind::RBrace)?;
                }
            }
            let record_span = self.stream.make_span(start);

            // E049: Check for duplicate field names
            let mut seen_names = std::collections::HashSet::new();
            for field in &fields {
                if !seen_names.insert(field.name.name.as_str()) {
                    return Err(ParseError::duplicate_field_name(
                        field.name.name.clone(),
                        field.span,
                    ));
                }
            }

            // Check for inline refinement: { x: Int, y: Int } { self.x > 0, self.y > 0 }
            if self.stream.check(&TokenKind::LBrace) {
                // Lookahead: check if this looks like a refinement predicate
                let checkpoint = self.stream.position();
                self.stream.advance(); // consume {

                let looks_like_refinement = match self.stream.peek_kind() {
                    // Handle 'self' for record refinements: { self.x > 0 }
                    Some(TokenKind::SelfValue) => true,
                    // Handle comparison operators for shorthand: { > 0 }
                    Some(TokenKind::Gt)
                    | Some(TokenKind::Lt)
                    | Some(TokenKind::GtEq)
                    | Some(TokenKind::LtEq)
                    | Some(TokenKind::EqEq)
                    | Some(TokenKind::BangEq)
                    | Some(TokenKind::Bang) => true,
                    // Handle 'it' keyword for the implicit value
                    Some(TokenKind::Ident(name)) if name.as_str() == "it" => true,
                    // Handle 'result' for return type refinements
                    Some(TokenKind::Result) => true,
                    // Handle lambda refinements: { |x| ... }
                    Some(TokenKind::Pipe) => true,
                    // Named predicates: { n: n > 0 }
                    // Or bare field references: { x >= 0.0, y >= 0.0 }
                    Some(TokenKind::Ident(_)) => {
                        // Check for named predicate pattern: ident ':'
                        // Or bare field/variable followed by comparison/operator
                        matches!(self.stream.peek_nth(1).map(|t| &t.kind),
                            Some(TokenKind::Colon)
                            | Some(TokenKind::GtEq) | Some(TokenKind::LtEq)
                            | Some(TokenKind::Gt) | Some(TokenKind::Lt)
                            | Some(TokenKind::EqEq) | Some(TokenKind::BangEq)
                            | Some(TokenKind::Dot) | Some(TokenKind::LParen)
                            | Some(TokenKind::Plus) | Some(TokenKind::Minus)
                            | Some(TokenKind::Star) | Some(TokenKind::Slash)
                            | Some(TokenKind::Percent)
                            | Some(TokenKind::AmpersandAmpersand) | Some(TokenKind::PipePipe)
                        )
                    }
                    // Boolean literals as refinements: { true }
                    Some(TokenKind::True) | Some(TokenKind::False) => true,
                    // Handle grouped expressions: { (self.x * self.x + ...).abs() < 0.001 }
                    Some(TokenKind::LParen) => true,
                    _ => false,
                };

                self.stream.reset_to(checkpoint);

                if looks_like_refinement {
                    // Parse the refinement predicate
                    let predicate = self.parse_refinement_predicate()?;

                    // Create the base record type
                    let base_type = Type::new(
                        TypeKind::Record { fields: fields.into_iter().collect(), row_var: Maybe::None },
                        record_span,
                    );

                    // Create the refined type
                    let refined_span = self.stream.make_span(start);
                    let refined_type = Type::new(
                        TypeKind::Refined {
                            base: Box::new(base_type),
                            predicate: Box::new(predicate),
                        },
                        refined_span,
                    );

                    return Ok(TypeDeclBody::Alias(refined_type));
                }
            }

            return Ok(TypeDeclBody::Record(fields.into_iter().collect()));
        }

        // Tuple or Dependent Pair: (T, U) or (n: T, arr: [T; n])
        // Spec: grammar/verum.ebnf line 404 - tuple_type
        // Spec: grammar/verum.ebnf line 443 - sigma_type for dependent pairs
        if self.stream.check(&TokenKind::LParen) {
            let start = self.stream.position();
            self.stream.advance();

            // Check for unit type
            if self.stream.check(&TokenKind::RParen) {
                self.stream.advance();
                // Unit type - treat as alias to Unit
                return Ok(TypeDeclBody::Unit);
            }

            // Look ahead to check if this is a dependent pair: (name: Type, ...)
            // If we see Ident followed by Colon, it's a dependent pair
            let is_dependent_pair = matches!(
                (self.stream.peek_kind(), self.stream.peek_nth(1).map(|t| &t.kind)),
                (Some(TokenKind::Ident(_)), Some(TokenKind::Colon))
            );

            if is_dependent_pair {
                // Parse dependent pair (sigma type): (n: Type1, arr: Type2)
                // Stored as an Alias to a sigma/dependent tuple type
                let types = self.comma_separated(|p| p.parse_sigma_type())?;
                self.stream.expect(TokenKind::RParen)?;

                // Convert to a SigmaTuple type body
                // This represents (n: Int, arr: [Int; n]) as a dependent pair
                let span = self.stream.make_span(start);
                return Ok(TypeDeclBody::SigmaTuple(types.into_iter().collect()));
            }

            // Parse regular types (anonymous tuple)
            let types = self.comma_separated(|p| p.parse_type())?;
            self.stream.expect(TokenKind::RParen)?;
            let tuple_span = self.stream.make_span(start);

            // Check for inline refinement: (Int, Int) { self.0 <= self.1 }
            // Use lookahead to distinguish refinement from record body
            if self.stream.check(&TokenKind::LBrace) {
                // Lookahead: check if this looks like a refinement predicate
                // Refinements typically start with: self, it, result, comparison operators, or identifiers with dots
                let checkpoint = self.stream.position();
                self.stream.advance(); // consume {

                let looks_like_refinement = match self.stream.peek_kind() {
                    // Handle 'self' for tuple refinements: { self.0 <= self.1 }
                    Some(TokenKind::SelfValue) => true,
                    // Handle comparison operators for shorthand: { > 0 }, { >= 0, <= 100 }
                    Some(TokenKind::Gt)
                    | Some(TokenKind::Lt)
                    | Some(TokenKind::GtEq)
                    | Some(TokenKind::LtEq)
                    | Some(TokenKind::EqEq)
                    | Some(TokenKind::BangEq)
                    | Some(TokenKind::Bang) => true,
                    // Handle 'it' keyword for the implicit value
                    Some(TokenKind::Ident(name)) if name.as_str() == "it" => true,
                    // Handle 'result' for return type refinements
                    Some(TokenKind::Result) => true,
                    // Handle lambda refinements: { |x| ... }
                    Some(TokenKind::Pipe) => true,
                    // Named predicates: { n: n > 0 }
                    Some(TokenKind::Ident(_)) => {
                        // Check for named predicate pattern: ident ':'
                        matches!(self.stream.peek_nth(1).map(|t| &t.kind), Some(TokenKind::Colon))
                    }
                    // Boolean literals as refinements: { true }
                    Some(TokenKind::True) | Some(TokenKind::False) => true,
                    // Handle grouped expressions: { (self.0 * self.0 + ...).abs() < 0.0001 }
                    Some(TokenKind::LParen) => true,
                    _ => false,
                };

                self.stream.reset_to(checkpoint);

                if looks_like_refinement {
                    // Parse the refinement predicate
                    let predicate = self.parse_refinement_predicate()?;

                    // Create the base tuple type
                    let base_type = Type::new(
                        TypeKind::Tuple(types.into_iter().collect()),
                        tuple_span,
                    );

                    // Create the refined type
                    let refined_span = self.stream.make_span(start);
                    let refined_type = Type::new(
                        TypeKind::Refined {
                            base: Box::new(base_type),
                            predicate: Box::new(predicate),
                        },
                        refined_span,
                    );

                    return Ok(TypeDeclBody::Alias(refined_type));
                }
            }

            // (T) is a newtype/tuple even with single element: `type Kilometers is (Float);`
            return Ok(TypeDeclBody::Tuple(types.into_iter().collect()));
        }

        // FIXED: Try to parse as a type expression first, then check if it's followed by |
        // This handles type aliases properly: type MyVec<T> is Vec<T>;
        // Use lookahead to distinguish between:
        // - Type alias: Vec<T>; (no pipe)
        // - Simple variant: Some | None
        // - Variant with data: Some(T) | None
        // - Variant with leading pipe: | Some(T) | None

        // Check for leading pipe (optional) - indicates variant type
        let has_leading_pipe = self.stream.consume(&TokenKind::Pipe).is_some();

        if has_leading_pipe {
            // Definitely a variant type - parse all variants
            let mut variants = Vec::new();
            let start_pos = self.stream.position();
            loop {
                // Safety: prevent infinite loop
                if !self.tick() || self.is_aborted() {
                    break;
                }

                // E086: Check for double pipe (empty variant)
                if self.stream.check(&TokenKind::Pipe) {
                    return Err(ParseError::empty_variant_pipe(self.stream.current_span()));
                }

                variants.push(self.parse_variant()?);
                if self.stream.consume(&TokenKind::Pipe).is_none() {
                    break;
                }
                // GRAMMAR: variant_list = [ '|' ] , variant , { '|' , variant } ;
                // After consuming '|', another variant is REQUIRED.
                if self.stream.check(&TokenKind::Semicolon) || self.stream.check(&TokenKind::Where)
                {
                    return Err(ParseError::trailing_separator(
                        "|",
                        "variant list",
                        self.stream.current_span(),
                    ));
                }
            }
            return Ok(TypeDeclBody::Variant(variants.into_iter().collect()));
        }

        // Try to parse as a variant if it looks like one (identifier without generic args or function syntax)
        if self.looks_like_variant() {
            let first_variant = self.parse_variant()?;

            if self.stream.check(&TokenKind::Pipe) || self.stream.check(&TokenKind::Comma) {
                // It's a variant type (accept both `|` and `,` as separators)
                let start_pos = self.stream.position();
                let mut variants = vec![first_variant];
                while self.stream.consume(&TokenKind::Pipe).is_some()
                    || self.stream.consume(&TokenKind::Comma).is_some()
                {
                    // Safety: prevent infinite loop
                    if !self.tick() || self.is_aborted() {
                        break;
                    }
                    // GRAMMAR: variant_list = [ '|' ] , variant , { '|' , variant } ;
                    // Also accepts commas as separators for convenience.
                    // After consuming separator, another variant is REQUIRED.
                    // Reject trailing separator before semicolon or where clause
                    if self.stream.check(&TokenKind::Semicolon)
                        || self.stream.check(&TokenKind::Where)
                    {
                        return Err(ParseError::trailing_separator(
                            "|",
                            "variant list",
                            self.stream.current_span(),
                        ));
                    }
                    // E086: Check for double pipe (empty variant)
                    if self.stream.check(&TokenKind::Pipe) {
                        return Err(ParseError::empty_variant_pipe(self.stream.current_span()));
                    }
                    variants.push(self.parse_variant()?);
                }
                return Ok(TypeDeclBody::Variant(variants.into_iter().collect()));
            }

            // Single variant without pipe - convert to type alias if it's just a name
            if first_variant.data.is_none() {
                // Convert back to type alias
                let ty = Type::new(
                    verum_ast::TypeKind::Path(Path::from_ident(first_variant.name)),
                    first_variant.span,
                );
                return Ok(TypeDeclBody::Alias(ty));
            } else {
                // Single variant with data - could be newtype
                return Ok(TypeDeclBody::Variant(
                    vec![first_variant].into_iter().collect(),
                ));
            }
        }

        // Parse as type alias (handles complex types like Vec<T>, fn(Int) -> Int, Int{> 0}, &Int)
        let ty = self.parse_type()?;

        // Check if this is a sigma-form refinement followed by comma -
        // indicates sigma-bindings (e.g. `type SizedVec is n: Int, data: [Int; n];`).
        // Post canonicalisation, the sigma surface form parses as
        // `TypeKind::Refined` with `predicate.binding = Some(name)`; we detect
        // that shape here.
        let looks_like_sigma_binding = matches!(
            &ty.kind,
            TypeKind::Refined { predicate, .. }
                if matches!(predicate.binding, verum_common::Maybe::Some(_))
        );
        if looks_like_sigma_binding && self.stream.check(&TokenKind::Comma) {
            // We have a sigma-binding followed by comma - parse additional sigma bindings
            let mut bindings = vec![ty];
            while self.stream.consume(&TokenKind::Comma).is_some() {
                let sigma = self.parse_sigma_type()?;
                bindings.push(sigma);
            }
            return Ok(TypeDeclBody::SigmaTuple(bindings.into_iter().collect()));
        }

        // Handle double-is pattern: `type Apply<F<_>, A> is F<A> is ();`
        // The second `is` provides a witness/representation type for type-level functions.
        // We consume it and ignore the representation, returning the alias as-is.
        // IMPORTANT: Only consume if the `is` is on the same line or followed by a type,
        // not if it's the start of a new declaration.
        if self.stream.check(&TokenKind::Is) {
            // Peek ahead: only consume if followed by something that looks like a type
            // (paren, identifier, etc.) NOT if followed by a keyword like `fn`, `type`, etc.
            let next_after_is = self.stream.peek_nth(1).map(|t| t.kind.clone());
            let looks_like_repr = matches!(
                next_after_is,
                Some(TokenKind::LParen) | Some(TokenKind::Ident(_)) | Some(TokenKind::Bang)
                | Some(TokenKind::LBracket) | Some(TokenKind::Ampersand)
            );
            if looks_like_repr {
                self.stream.advance(); // consume 'is'
                let _repr_ty = self.parse_type()?;
            }
        }

        Ok(TypeDeclBody::Alias(ty))
    }

    /// Check if the current position looks like a variant (not a complex type).
    /// Variants start with a simple identifier, not with complex type syntax.
    /// Decide whether a leading `@` opens an attribute on a variant
    /// (`@serialize(...) Ok`) or a meta-type alias body (`@builtin_path`,
    /// `@Expr`).
    ///
    /// Walk past `@X` and an optional `(…)` arg list, then peek. If an
    /// identifier (the variant's own name) follows, this is an attribute
    /// on a variant; otherwise — `;`, `,`, `|`, `)`, `where`, EOF — the
    /// `@X` itself is the whole body: a meta type alias like
    /// `type Path<A> is @builtin_path;`.
    fn at_prefix_looks_like_variant_attr(&self) -> bool {
        // Walk past any number of stacked attributes:
        //   attribute chain:  ( @  Ident  [ ( … ) ] )+  VariantName
        //   meta-type body :    @  Ident                (then ; , | ) where …)
        //
        // A variant name may be lexed as a keyword (`Ok`, `Err`, `Some`,
        // `None`, `Result`, `Await`, etc.) — matching
        // `consume_ident_or_keyword` in `parse_variant`. Accept any token
        // that could start a variant name.
        fn is_variant_start(t: Option<&TokenKind>) -> bool {
            matches!(
                t,
                Some(TokenKind::Ident(_))
                    | Some(TokenKind::Some)
                    | Some(TokenKind::None)
                    | Some(TokenKind::Ok)
                    | Some(TokenKind::Err)
                    | Some(TokenKind::Result)
                    | Some(TokenKind::Await)
                    | Some(TokenKind::Yield)
            )
        }

        let mut i = 0usize;
        loop {
            // Must start with `@`
            if !matches!(self.stream.peek_nth(i).map(|t| &t.kind), Some(TokenKind::At)) {
                break;
            }
            i += 1;
            // Attribute name (ident-or-keyword).
            if !is_variant_start(self.stream.peek_nth(i).map(|t| &t.kind)) {
                return false;
            }
            i += 1;
            // Optional `(args)` — balanced-paren skip.
            if matches!(self.stream.peek_nth(i).map(|t| &t.kind), Some(TokenKind::LParen)) {
                let mut depth = 1i32;
                i += 1;
                while depth > 0 {
                    match self.stream.peek_nth(i).map(|t| &t.kind) {
                        Some(TokenKind::LParen) => depth += 1,
                        Some(TokenKind::RParen) => depth -= 1,
                        None => return false,
                        _ => {}
                    }
                    i += 1;
                }
            }
            // Continue the loop: another `@` begins a stacked attribute;
            // anything else is the variant name (or meta-body terminator).
        }
        is_variant_start(self.stream.peek_nth(i).map(|t| &t.kind))
    }

    fn looks_like_variant(&self) -> bool {
        // Variant patterns:
        // - Simple: Name
        // - Tuple: Name(...)
        // - Record: Name { ... }
        //
        // Non-variant patterns (type aliases):
        // - Function type: fn(...)
        // - Reference: &Type, %Type, *const Type
        // - Generic with path: Vec<T>, std.collections.Map<K, V>
        // - Refinement without name: Int{> 0}

        // Check for non-variant patterns
        match self.stream.peek_kind() {
            Some(TokenKind::Fn) => false,        // fn(Int) -> Int
            Some(TokenKind::Async) => false,     // async fn(Int) -> Int
            Some(TokenKind::Extern) => false,    // extern "C" fn(...) - FFI function pointer
            Some(TokenKind::Ampersand) => false, // &Int, &mut Int
            Some(TokenKind::Percent) => false,   // %Int
            Some(TokenKind::Star) => false,      // *const Int
            Some(TokenKind::Implement) => false, // impl Display
            // `@`-prefixed tokens are either an attribute on a variant
            // (`@serialize(...) Ok | @deprecated Legacy`) or a meta-type
            // alias body (`is @builtin_path;`). Use the helper to walk
            // past the attribute and peek.
            Some(TokenKind::At) => self.at_prefix_looks_like_variant_attr(),
            Some(TokenKind::Ident(name)) if name.as_str() == "impl" => false, // impl Display (contextual keyword)
            Some(TokenKind::Ident(name)) if name.as_str() == "dyn" => false,  // dyn Display
            Some(TokenKind::Ident(name)) if name.as_str() == "some" => false, // some T: Bound (existential type)
            Some(TokenKind::LBracket) => false,                               // [T] or [T; N]
            Some(TokenKind::Ident(_)) => {
                // Could be a variant or a type alias
                // Check if followed by < (generic args) or { (refinement) at the same level
                // If so, it's likely a type alias like Vec<T> or Int{> 0}
                // If followed by nothing, ( or {, it's a variant

                match self.stream.peek_nth(1).map(|t| &t.kind) {
                    Some(TokenKind::Lt) => {
                        // Could be Vec<T> (type alias) or variant with generic args
                        // Verum variants don't have generic args in the type body,
                        // so this is a type alias
                        false
                    }
                    Some(TokenKind::Dot) => {
                        // Path like std.collections.Vec - type alias
                        false
                    }
                    Some(TokenKind::Colon) => {
                        // Sigma type: x: Int where x > 0 - type alias
                        false
                    }
                    Some(TokenKind::Where) => {
                        // Type with refinement: Int where x > 0 - type alias
                        false
                    }
                    Some(TokenKind::With) => {
                        // Capability-restricted type: Database with [Read] - type alias
                        false
                    }
                    Some(TokenKind::LBrace) => {
                        // Could be:
                        // 1. Refinement type: Int{> 0}, Port{> 0}, Duration{> 0} (type alias)
                        // 2. Record variant: Error { code: Int }
                        //
                        // Better heuristic: Look at what follows the brace.
                        // If it's a comparison operator, it's a refinement type.
                        // If it's an identifier followed by ':', it's a record variant.
                        match self.stream.peek_nth(2).map(|t| &t.kind) {
                            // Comparison operators indicate refinement: Type{> 0}, Type{>= 0}
                            Some(TokenKind::Gt)
                            | Some(TokenKind::Lt)
                            | Some(TokenKind::GtEq)
                            | Some(TokenKind::LtEq)
                            | Some(TokenKind::EqEq)
                            | Some(TokenKind::BangEq) => false,
                            // Pipe after { suggests lambda refinement: Type{|x| x > 0}
                            Some(TokenKind::Pipe) => false,
                            // Empty braces could be either - conservatively assume record
                            Some(TokenKind::RBrace) => true,
                            // Identifier might be:
                            // - Record field: Error { code: Int } (ident followed by : then TYPE)
                            // - Refinement expression: Type{is_valid} (ident not followed by :)
                            // - Named refinement: Int { min: self >= 0 } (ident : EXPRESSION)
                            Some(TokenKind::Ident(_)) => {
                                // Check if the next token after identifier is ':'
                                if !matches!(
                                    self.stream.peek_nth(3).map(|t| &t.kind),
                                    Some(TokenKind::Colon)
                                ) {
                                    // No colon - it's a refinement expression like {is_valid}
                                    return false;
                                }
                                // Has colon - distinguish record field from named refinement
                                // Token positions: 0=TypeName, 1={, 2=field/name, 3=:, 4=value, 5=...
                                // Record field: `{ code: Int }` - token 4 is a type name, token 5 is , or }
                                // Named refinement: `{ min: self >= 0 }` - token 4 might be `self`/`it`
                                //                   or token 5 is an operator like >= < > + - etc.

                                // Check for 'self' keyword at token 4 - indicates refinement
                                if matches!(self.stream.peek_nth(4).map(|t| &t.kind), Some(TokenKind::SelfValue)) {
                                    return false; // Named refinement with 'self'
                                }

                                // Check for "it" identifier at token 4 - indicates refinement
                                if let Some(TokenKind::Ident(name)) = self.stream.peek_nth(4).map(|t| &t.kind) {
                                    if name.as_str() == "it" {
                                        return false; // Named refinement with 'it'
                                    }
                                }

                                // Check token 5 to decide record-field vs named-refinement.
                                //
                                // Tok4 is a plain identifier (not `self`/`it` — those
                                // returned false already).
                                //
                                // Unambiguously operator-shaped at tok5 → refinement:
                                //   `>=`, `<=`, `==`, `!=`, arith, `&&`, `||`
                                //
                                // Ambiguous at tok5 (`<`, `>`, `.`) — could be generic
                                // type args (`Maybe<Int>`, `module.Type`) OR comparison
                                // / method call on a value (`x < 0`, `x > 0`, `x.foo()`).
                                // Disambiguate by peeking tok6:
                                //   integer/float/minus literal → value comparison
                                //   identifier starting with uppercase → type
                                //   identifier starting with lowercase → value
                                //
                                // This keeps generic-type record-variant fields parsing
                                // (the repro in `variant_record_maybe_bug.rs`) while
                                // letting named refinements with `<`/`>` predicates
                                // (`Int { x: x < 0 }`) route through the refinement
                                // parser.
                                if let Some(tok5) = self.stream.peek_nth(5).map(|t| &t.kind) {
                                    match tok5 {
                                        // Always-refinement operators:
                                        TokenKind::GtEq | TokenKind::LtEq | TokenKind::EqEq |
                                        TokenKind::BangEq |
                                        TokenKind::Plus | TokenKind::Minus | TokenKind::Star |
                                        TokenKind::Slash | TokenKind::Percent |
                                        TokenKind::AmpersandAmpersand | TokenKind::PipePipe => {
                                            return false; // Named refinement predicate
                                        }
                                        // Ambiguous — disambiguate on tok6:
                                        TokenKind::Lt | TokenKind::Gt | TokenKind::Dot => {
                                            match self.stream.peek_nth(6).map(|t| &t.kind) {
                                                // Value literal → comparison/arith → refinement
                                                Some(TokenKind::Integer(_))
                                                | Some(TokenKind::Float(_))
                                                | Some(TokenKind::Minus)
                                                | Some(TokenKind::True)
                                                | Some(TokenKind::False) => return false,
                                                // Identifier: uppercase=Type (variant),
                                                // lowercase=value (refinement). Tok5=`.`
                                                // additionally disambiguates: after a
                                                // value-ident, `.method()` is a value-
                                                // expression — a refinement predicate.
                                                Some(TokenKind::Ident(name)) => {
                                                    let is_type_like = name
                                                        .as_str()
                                                        .chars()
                                                        .next()
                                                        .map(|c| c.is_ascii_uppercase())
                                                        .unwrap_or(false);
                                                    if !is_type_like {
                                                        return false; // value → refinement
                                                    }
                                                    // Uppercase ident after `.` could be
                                                    // a qualified TYPE path; after `<`
                                                    // it's a generic type arg. Either way,
                                                    // record-variant field.
                                                }
                                                _ => {}
                                            }
                                        }
                                        _ => {}
                                    }
                                }

                                // Default: assume it's a record variant field
                                true
                            }
                            // Anything else - assume type alias with refinement
                            _ => false,
                        }
                    }
                    _ => true, // Simple identifier - could be variant
                }
            }
            _ => true,
        }
    }

    /// Parse a variant.
    fn parse_variant(&mut self) -> ParseResult<Variant> {
        // (Companion fn `path_endpoint_depth` defined at module
        // scope below, used by this fn to compute n-cell dim.)
        let start_pos = self.stream.position();

        // Parse optional attributes: @attr VariantName(...)
        let attributes = self.parse_attributes()?;

        // Variant constructors live in their own namespace (`Type.Variant`)
        // and cannot collide with reserved keywords used elsewhere, so
        // we accept *any* keyword as a variant name. This lets the
        // HoTT stdlib name HIT path constructors `loop`, `merid`,
        // `trunc_path`, `push`, etc. — following the canonical names
        // in Kapulkin–Lumsdaine and Univalent Foundations without
        // having to rename them for our lexer's benefit.
        let name = self.consume_ident_or_any_keyword()?;
        let name_span = self.stream.current_span();

        // Parse optional generic parameters on variant (GADT constructors):
        // | IntLit<T>(Int) where T == Int
        // | FZero<N: meta Nat> where N > 0
        let generic_params: Vec<_> = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?.into_iter().collect()
        } else {
            Vec::new()
        };

        // Parse variant data (tuple, record, or discriminant)
        let data;
        // HIT path-constructor endpoint metadata: `Foo(args) = from..to`
        let mut path_endpoints: Maybe<(
            verum_common::Heap<verum_ast::Expr>,
            verum_common::Heap<verum_ast::Expr>,
        )> = Maybe::None;

        // Tuple variant: Some(T), or HIT path constructor: Seg() = Zero..One
        if self.stream.check(&TokenKind::LParen) {
            self.stream.advance();
            let types = if self.stream.check(&TokenKind::RParen) {
                Vec::new()
            } else {
                self.comma_separated(|p| p.parse_type())?
            };
            self.stream.expect(TokenKind::RParen)?;

            // HIT path constructor: `Foo(args) = from..to`
            //
            // Parser captures the endpoint expressions into the
            // variant's `path_endpoints` slot. The lowering to
            // `Type::HigherInductive` emits a `PathConstructor` when
            // these endpoints are present.
            if self.stream.check(&TokenKind::Eq) {
                let cp = self.stream.position();
                self.stream.advance(); // consume `=`
                if let Ok(expr) = self.parse_expr_no_struct() {
                    // peel one
                    // optional outer Paren so users can write
                    // `= (a..b)` for 1-cells (just as readable as
                    // `= a..b`). For n-cells the user always
                    // writes `(a..b)..(c..d)` whose top level is
                    // a Range; the parens around endpoints
                    // survive on lhs/rhs so the depth probe can
                    // count them.
                    let unwrapped = match &expr.kind {
                        verum_ast::ExprKind::Paren(inner) => (**inner).clone(),
                        _ => expr.clone(),
                    };
                    if let verum_ast::ExprKind::Range {
                        start: verum_common::Maybe::Some(lhs),
                        end: verum_common::Maybe::Some(rhs),
                        ..
                    } = &unwrapped.kind
                    {
                        path_endpoints = Maybe::Some((lhs.clone(), rhs.clone()));
                        data = Maybe::Some(VariantData::Tuple(types.into_iter().collect()));
                    } else {
                        // Not a path constructor — restore and treat as tuple.
                        self.stream.reset_to(cp);
                        data = Maybe::Some(VariantData::Tuple(types.into_iter().collect()));
                    }
                } else {
                    self.stream.reset_to(cp);
                    data = Maybe::Some(VariantData::Tuple(types.into_iter().collect()));
                }
            } else {
                data = Maybe::Some(VariantData::Tuple(types.into_iter().collect()));
            }
        }
        // Record variant: Error { code: Int }
        else if self.stream.check(&TokenKind::LBrace) {
            self.stream.advance();
            let fields = if self.stream.check(&TokenKind::RBrace) {
                Vec::new()
            } else {
                self.comma_separated(|p| p.parse_record_field())?
            };
            self.stream.expect(TokenKind::RBrace)?;

            // E049: Check for duplicate field names
            let mut seen_names = std::collections::HashSet::new();
            for field in &fields {
                if !seen_names.insert(field.name.name.as_str()) {
                    return Err(ParseError::duplicate_field_name(
                        field.name.name.clone(),
                        field.span,
                    ));
                }
            }

            data = Maybe::Some(VariantData::Record(fields.into_iter().collect()));
        }
        // Discriminant variant: Red = 0
        else if self.stream.check(&TokenKind::Eq) {
            self.stream.advance();
            let _discriminant = self.parse_prefix_expr()?;
            data = Maybe::None;
        }
        // Unit variant: None
        else {
            data = Maybe::None;
        }

        // HIT path-constructor endpoint type (second form):
        // `| loop: Path<S1>(base, base);`
        // `| merid(a: A): Path<Susp<A>>(north, south);`
        // `| push(c: C): Path<Pushout<A, B, C>(f, g)>(inl(f(c)), inr(g(c)));`
        //
        // The stdlib `core/math/hott.vr` encodes Higher Inductive Type
        // path constructors with a trailing type annotation after the
        // variant's payload. Semantically the annotation is the
        // variant's `Path<Carrier>(lhs, rhs)` identity type, which is
        // equivalent to the range form `Seg() = lhs..rhs` already
        // supported above. Here we accept the type-annotation spelling
        // by pulling the `lhs`/`rhs` expressions out of the parsed
        // PathType / DependentApp node and populating `path_endpoints`.
        // Non-`Path<…>(…)` annotations are preserved but left without
        // endpoint metadata — they still document the constructor's
        // type for downstream HoTT verification passes.
        if path_endpoints.is_none() && self.stream.check(&TokenKind::Colon) {
            let cp = self.stream.position();
            self.stream.advance(); // consume `:`
            if let Some(ty) = self.optional(|p| p.parse_type_no_refinement()) {
                match &ty.kind {
                    // `Path<Carrier>(a, b)` — direct sugar
                    verum_ast::ty::TypeKind::PathType { lhs, rhs, .. } => {
                        path_endpoints = Maybe::Some((
                            verum_common::Heap::new((**lhs).clone()),
                            verum_common::Heap::new((**rhs).clone()),
                        ));
                    }
                    // `DependentApp { carrier=Path<C>, value_args=[a, b] }` —
                    // the generalised DependentApp form also produces path
                    // constructors when exactly two value args are present
                    // and the inner carrier is a Path.
                    verum_ast::ty::TypeKind::DependentApp { value_args, .. }
                        if value_args.len() == 2 =>
                    {
                        let lhs_expr = value_args.iter().next().unwrap().clone();
                        let rhs_expr = value_args.iter().nth(1).unwrap().clone();
                        path_endpoints = Maybe::Some((
                            verum_common::Heap::new(lhs_expr),
                            verum_common::Heap::new(rhs_expr),
                        ));
                    }
                    _ => {
                        // Not a recognised HIT index form. Keep parsing but
                        // don't emit path_endpoints — the type annotation
                        // survives only in documentation.
                    }
                }
            } else {
                // Couldn't parse as a type — roll back and let the caller
                // see the `:` (it's likely a syntax error that should be
                // reported at a higher level).
                self.stream.reset_to(cp);
            }
        }

        // Parse optional where clause on variant (GADT constraints):
        // | IntLit(Int) where T == Int
        // | Zero where N > 0
        let where_clause = if self.stream.check(&TokenKind::Where) {
            // Parse a simplified where clause for variants:
            // We parse predicates until we hit '|', ';', or '}'
            // IMPORTANT: We use parse_variant_where_expr instead of parse_expr
            // because parse_expr would consume '|' as bitwise OR operator.
            let where_start = self.stream.position();
            self.stream.advance(); // consume 'where'

            let mut predicates = Vec::new();
            loop {
                if !self.tick() || self.is_aborted() {
                    break;
                }
                // Stop at variant/type terminators
                if self.stream.check(&TokenKind::Pipe)
                    || self.stream.check(&TokenKind::Semicolon)
                    || self.stream.check(&TokenKind::RBrace)
                    || self.stream.at_end()
                {
                    break;
                }
                // Parse a variant where constraint expression that stops before '|'
                // Uses min_bp=8 to avoid consuming '|' (which has left_bp=7)
                // but still handles comparison operators through explicit two-sided parsing
                let constraint = self.parse_variant_where_constraint()?;
                let pred_span = constraint.span;
                predicates.push(WherePredicate {
                    kind: WherePredicateKind::Meta { constraint },
                    span: pred_span,
                });
                // Consume optional comma between predicates
                if self.stream.consume(&TokenKind::Comma).is_none() {
                    break;
                }
            }
            let where_span = self.stream.make_span(where_start);
            Maybe::Some(WhereClause::new(predicates.into_iter().collect(), where_span))
        } else {
            Maybe::None
        };

        let span = self.stream.make_span(start_pos);
        // n-cell dimension
        // detection. For a 1-cell variant `= a..b` both endpoints
        // are bare expressions ⇒ dim = 1. For an n-cell variant
        // both endpoints are paren-wrapped ranges (one dimension
        // up each). The dim is the max of the lhs / rhs nesting
        // depths plus 1 (since the range itself is one dimension).
        let path_dim: u32 = match &path_endpoints {
            Maybe::Some((lhs, rhs)) => {
                let d = path_endpoint_depth(lhs).max(path_endpoint_depth(rhs)) + 1;
                d
            }
            Maybe::None => 1,
        };
        Ok(Variant {
            name: Ident::new(name, name_span),
            generic_params: generic_params.into_iter().collect(),
            data,
            where_clause,
            attributes: attributes.into_iter().collect(),
            path_endpoints,
            path_dim,
            span,
        })
    }

    /// Parse a variant where clause constraint expression.
    /// This is like parse_expr but stops before '|' (which has left_bp=7 as BitOr).
    /// GADT where clauses contain comparisons like `T == Int`, `N > 0`.
    fn parse_variant_where_constraint(&mut self) -> ParseResult<Expr> {
        // (Companion: see `path_endpoint_depth` below for the n-cell
        // dimension-detection helper used by the variant parser.)
        // Parse LHS - use bp=8 to stop before '|' (left_bp=7)
        let lhs = self.parse_expr_bp(8)?;

        // Check for comparison operator
        let op_kind = match self.stream.peek_kind() {
            Some(TokenKind::EqEq) | Some(TokenKind::BangEq)
            | Some(TokenKind::Lt) | Some(TokenKind::Gt)
            | Some(TokenKind::LtEq) | Some(TokenKind::GtEq) => {
                match self.stream.peek_kind().cloned() {
                    Some(kind) => kind,
                    None => return Ok(lhs),
                }
            }
            _ => return Ok(lhs),
        };

        self.stream.advance(); // consume operator
        let rhs = self.parse_expr_bp(8)?;

        let op = match op_kind {
            TokenKind::EqEq => verum_ast::BinOp::Eq,
            TokenKind::BangEq => verum_ast::BinOp::Ne,
            TokenKind::Lt => verum_ast::BinOp::Lt,
            TokenKind::Gt => verum_ast::BinOp::Gt,
            TokenKind::LtEq => verum_ast::BinOp::Le,
            TokenKind::GtEq => verum_ast::BinOp::Ge,
            _ => unreachable!(),
        };

        let span = lhs.span.merge(rhs.span);
        Ok(Expr::new(
            ExprKind::Binary {
                left: Box::new(lhs),
                op,
                right: Box::new(rhs),
            },
            span,
        ))
    }

    /// Parse record fields.
    ///
    /// Grammar: field = { attribute } , [ visibility ] , identifier , ':' , type_expr , [ field_default ] ;
    ///          field_default = '=' , expression ;
    ///
    /// Fields can have default values: `field: Type = default_expr` for builder pattern support.
    fn parse_record_field(&mut self) -> ParseResult<RecordField> {
        let start_pos = self.stream.position();

        // Parse optional attributes: @attr field_name: Type
        let attributes = self.parse_attributes()?;

        // E046: Invalid record field - literal instead of field name
        if let Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_)) | Some(TokenKind::Text(_)) =
            self.stream.peek_kind()
        {
            return Err(ParseError::invalid_record_field(
                "expected field name, found literal",
                self.stream.current_span(),
            ));
        }

        let vis = self.parse_visibility()?;

        // Handle `ghost` keyword prefix on fields: `ghost field_name: Type`
        // This is equivalent to `@ghost field_name: Type`
        let is_ghost = if let Some(TokenKind::Ident(name)) = self.stream.peek_kind() {
            if name.as_str() == "ghost" {
                // Peek ahead to see if this is `ghost name:` (ghost field) or `ghost:` (field named ghost)
                if let Some(Token { kind: TokenKind::Ident(_), .. }) = self.stream.peek_nth(1) {
                    self.stream.advance(); // consume `ghost`
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };
        let _ = is_ghost; // ghost fields are parsed but ghost annotation is informational

        let name = self.consume_ident_or_keyword()?;
        let name_span = self.stream.current_span();

        // E046: Missing colon - identifier followed by identifier (e.g., `x y: Int`)
        if self.is_ident() && !self.stream.check(&TokenKind::Colon) {
            return Err(ParseError::invalid_record_field(
                "expected ':' after field name",
                self.stream.current_span(),
            ));
        }

        // E047: Missing field type - check if ',' or '}' instead of ':'
        if self.stream.check(&TokenKind::Comma) || self.stream.check(&TokenKind::RBrace) {
            return Err(ParseError::missing_field_type(self.stream.current_span()));
        }

        // E046: Double colon - `x:: Int`
        if self.stream.check(&TokenKind::ColonColon) {
            return Err(ParseError::invalid_record_field(
                "invalid double colon '::' in field",
                self.stream.current_span(),
            ));
        }

        self.stream.expect(TokenKind::Colon)?;

        // E046: Missing type after colon - `x: }`
        if self.stream.check(&TokenKind::RBrace) || self.stream.check(&TokenKind::Comma) {
            return Err(ParseError::invalid_record_field(
                "missing type after ':'",
                self.stream.current_span(),
            ));
        }

        let ty = self.parse_type()?;

        // Parse optional default value: = expression
        // Spec: grammar/verum.ebnf - field_default
        let default_value = if self.stream.consume(&TokenKind::Eq).is_some() {
            Maybe::Some(self.parse_expr()?)
        } else {
            Maybe::None
        };

        // Extract BitSpec from @bits(N) and @offset(N) attributes if present
        let bit_spec = Self::extract_bit_spec_from_attributes(&attributes, start_pos);

        let span = self.stream.make_span(start_pos);
        Ok(RecordField {
            visibility: vis,
            name: Ident::new(name, name_span),
            ty,
            attributes: attributes.into_iter().collect(),
            default_value,
            bit_spec,
            span,
        })
    }

    /// Extract BitSpec from @bits(N) and @offset(N) attributes.
    ///
    /// This enables first-class bitfield support where field attributes define bit layout:
    /// ```verum
    /// @bitfield
    /// type Flags is {
    ///     @bits(1) carry: Bool,
    ///     @bits(1) zero: Bool,
    ///     @bits(6) reserved: UInt,
    /// };
    /// ```
    ///
    /// Bitfield system: `@bits(N)` sets bit width, `@offset(N)` sets bit offset.
    /// Used with `@repr(packed)` types for hardware register layouts and protocol headers.
    fn extract_bit_spec_from_attributes(
        attributes: &[verum_ast::Attribute],
        span_start: usize,
    ) -> Maybe<BitSpec> {
        let mut bit_width: Option<(u32, Span)> = None;
        let mut bit_offset: Option<u32> = None;

        for attr in attributes {
            let name = attr.name.as_str();
            match name {
                "bits" => {
                    // @bits(N) where N is an integer literal
                    if let Some(args) = &attr.args {
                        if let Some(first_arg) = args.first() {
                            if let ExprKind::Literal(lit) = &first_arg.kind {
                                if let LiteralKind::Int(int_lit) = &lit.kind {
                                    bit_width = Some((int_lit.value as u32, attr.span));
                                }
                            }
                        }
                    }
                }
                "offset" => {
                    // @offset(N) where N is an integer literal
                    if let Some(args) = &attr.args {
                        if let Some(first_arg) = args.first() {
                            if let ExprKind::Literal(lit) = &first_arg.kind {
                                if let LiteralKind::Int(int_lit) = &lit.kind {
                                    bit_offset = Some(int_lit.value as u32);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Only create BitSpec if @bits was specified
        if let Some((bits, span)) = bit_width {
            Maybe::Some(BitSpec {
                width: BitWidth { bits, span },
                offset: bit_offset.map(Maybe::Some).unwrap_or(Maybe::None),
                span,
            })
        } else {
            Maybe::None
        }
    }

    /// Parse protocol extends clause: extends Base1 + Base2<T> + Base3
    /// Spec: grammar/verum.ebnf:691 - trait_path = path , [ type_args ]
    /// Supports generic arguments: `extends Converter<A, B>`
    fn parse_protocol_extends(&mut self) -> ParseResult<Vec<Type>> {
        let mut types = vec![self.parse_type_no_refinement()?];

        while self.stream.consume(&TokenKind::Plus).is_some() {
            // Don't continue if we hit the opening brace or where clause
            if self.stream.check(&TokenKind::LBrace) || self.stream.check(&TokenKind::Where) {
                break;
            }
            types.push(self.parse_type_no_refinement()?);
        }

        Ok(types)
    }

    /// Parse protocol items.
    ///
    /// SAFETY: Includes forward progress check to prevent infinite loops
    /// on malformed input.
    fn parse_protocol_items(&mut self) -> ParseResult<Vec<ProtocolItem>> {
        let mut items = Vec::new();

        while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }

            let pos_before = self.stream.position();

            match self.parse_protocol_item() {
                Ok(item) => items.push(item),
                Err(e) => {
                    self.error(e);
                    self.synchronize();
                }
            }

            // CRITICAL FIX: Ensure forward progress to prevent infinite loop
            // If we didn't advance, force advancement to avoid hanging
            if self.stream.position() == pos_before && !self.stream.at_end() {
                self.stream.advance();
            }
        }

        Ok(items)
    }

    /// Parse a single protocol item.
    fn parse_protocol_item(&mut self) -> ParseResult<ProtocolItem> {
        let start_pos = self.stream.position();

        // Parse attributes first (e.g., @must_use, @deprecated, @doc)
        let _attributes = self.parse_attributes()?;

        // Check for 'default' contextual keyword before type/const/fn
        // 'default' is not a reserved keyword, so we check for it as an identifier
        let is_default = if let Some(TokenKind::Ident(name)) = self.stream.peek_kind() {
            if name == "default" {
                self.stream.advance();
                true
            } else {
                false
            }
        } else {
            false
        };

        // Associated type
        if self.stream.check(&TokenKind::Type) {
            self.stream.advance();
            // Use consume_ident_or_any_keyword to allow keywords like Err as type names
            let name = self.consume_ident_or_any_keyword()?;
            let name_span = self.stream.current_span();

            // Type parameters (for GATs)
            let type_params: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
                self.parse_generic_params()?
            } else {
                List::new()
            };

            // Bounds: type Item: Clone + Debug  OR  type Size: Int{> 0}
            let bounds = if self.stream.consume(&TokenKind::Colon).is_some() {
                // Try to parse as a type first (handles refinements like Int{> 0})
                // If we see a + after parsing, treat it as multiple protocol bounds
                let first_type = self.parse_type()?;

                // Check if this is a protocol bound list (Clone + Debug) or a single type constraint
                if self.stream.check(&TokenKind::Plus) {
                    // Multiple protocol bounds - convert type to path and continue
                    let mut bounds = vec![self.type_to_path(first_type)?];

                    while self.stream.consume(&TokenKind::Plus).is_some() {
                        if !self.is_ident()
                            && !self.stream.check(&TokenKind::SelfType)
                            && !self.stream.check(&TokenKind::Bang)
                        {
                            break;
                        }
                        bounds.push(self.parse_path()?);
                    }
                    bounds
                } else {
                    // Single type constraint - convert to path if possible
                    vec![self.type_to_path(first_type)?]
                }
            } else {
                Vec::new()
            };

            // Where clause (may come before or after default type)
            let mut where_clause = if self.stream.peek_kind() == Some(&TokenKind::Where) {
                Maybe::Some(self.parse_where_clause()?)
            } else {
                Maybe::None
            };

            // Default type: = Type  OR  is Type
            // Spec: grammar/verum.ebnf lines 862-863 (default_type = '=' , type_expr)
            // Also accept 'is' since Verum type definitions use 'is' syntax
            // e.g., `type Item is Int;` in protocol bodies
            let default_type = if self.stream.consume(&TokenKind::Eq).is_some()
                || self.stream.consume(&TokenKind::Is).is_some()
            {
                // Use no_refinement to avoid consuming `where` as a refinement predicate
                // e.g., `type Ordering is StandardOrdering where Self.Ordering: Eq;`
                Maybe::Some(self.parse_type_no_refinement()?)
            } else {
                Maybe::None
            };

            // Where clause can also come after default type with 'is' syntax
            // e.g., `type Ordering is StandardOrdering where Self.Ordering: Eq;`
            if where_clause.is_none() && self.stream.peek_kind() == Some(&TokenKind::Where) {
                where_clause = Maybe::Some(self.parse_where_clause()?);
            }

            // If 'default' keyword was used but no default type was provided, error
            if is_default && default_type.is_none() {
                return Err(ParseError::invalid_syntax(
                    "expected default type after 'default type' keyword (use '= Type')",
                    self.stream.current_span(),
                ));
            }

            self.stream.expect(TokenKind::Semicolon)?;

            let span = self.stream.make_span(start_pos);
            return Ok(ProtocolItem {
                kind: ProtocolItemKind::Type {
                    name: Ident::new(name, name_span),
                    type_params,
                    bounds: bounds.into_iter().collect(),
                    where_clause,
                    default_type,
                },
                span,
            });
        }

        // Associated constant
        if self.stream.check(&TokenKind::Const) {
            self.stream.advance();
            let name = self.consume_ident()?;
            let name_span = self.stream.current_span();

            self.stream.expect(TokenKind::Colon)?;
            let ty = self.parse_type()?;
            self.stream.expect(TokenKind::Semicolon)?;

            let span = self.stream.make_span(start_pos);
            return Ok(ProtocolItem {
                kind: ProtocolItemKind::Const {
                    name: Ident::new(name, name_span),
                    ty,
                },
                span,
            });
        }

        // Protocol-level axiom — T1-R model-theoretic semantics.
        //
        // Syntax: `axiom name<G>(params) [requires R] [ensures E] ;`
        // A protocol axiom becomes a proof obligation at every
        // `implement` site. The full parser for axiom declarations is
        // in proof.rs; we dispatch to it here and wrap the resulting
        // AxiomDecl in `ProtocolItemKind::Axiom`.
        if self.stream.check(&TokenKind::Axiom) {
            let axiom_item = self.parse_axiom(Vec::new(), verum_ast::decl::Visibility::Public)?;
            let item_span = self.stream.make_span(start_pos);
            if let ItemKind::Axiom(axiom_decl) = axiom_item.kind {
                return Ok(ProtocolItem {
                    kind: ProtocolItemKind::Axiom(axiom_decl),
                    span: item_span,
                });
            } else {
                return Err(ParseError::invalid_syntax(
                    "expected axiom declaration inside protocol body",
                    item_span,
                ));
            }
        }

        // Function (with optional pure, meta, async, unsafe and optional generator *)
        // Grammar: function_modifiers = [ 'pure' ] , [ meta_modifier ] , [ 'async' ] , [ 'unsafe' ]
        // Order: [pure] [meta | meta(N)] [async] [unsafe] fn [*] name
        // Staged metaprogramming: meta(N) defines function at stage N
        if self.stream.check(&TokenKind::Pure)
            || self.stream.check(&TokenKind::Meta)
            || self.stream.check(&TokenKind::Async)
            || self.stream.check(&TokenKind::Unsafe)
            || self.stream.check(&TokenKind::Fn)
        {
            let is_pure = self.stream.consume(&TokenKind::Pure).is_some();
            // Parse meta with optional stage level: meta or meta(N)
            let (is_meta, stage_level) = self.parse_meta_modifier()?;
            let is_async = self.stream.consume(&TokenKind::Async).is_some();
            let is_unsafe = self.stream.consume(&TokenKind::Unsafe).is_some();
            self.stream.expect(TokenKind::Fn)?;

            // Check for generator syntax: fn* or async fn*
            let is_generator = self.stream.consume(&TokenKind::Star).is_some();

            // E052: Missing function name - `fn;` or `fn(`
            if self.stream.check(&TokenKind::Semicolon)
                || self.stream.check(&TokenKind::LParen)
                || self.stream.check(&TokenKind::RBrace)
            {
                return Err(ParseError::invalid_protocol_method(
                    "missing function name after 'fn'",
                    self.stream.current_span(),
                ));
            }

            // Allow contextual keywords as function names (e.g., `show` which is a proof keyword)
            let name = self.consume_ident_or_keyword()?;
            let name_span = self.stream.current_span();

            // Generic parameters: fn foo<T, U>
            let generics: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
                self.parse_generic_params()?
            } else {
                List::new()
            };

            // E052: Check for invalid parameter (literal)
            if self.stream.check(&TokenKind::LParen) {
                if let Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_)) | Some(TokenKind::Text(_)) =
                    self.stream.peek_nth(1).map(|t| &t.kind)
                {
                    return Err(ParseError::invalid_protocol_method(
                        "invalid parameter: expected pattern, found literal",
                        self.stream.peek_nth(1).map(|t| t.span).unwrap_or(self.stream.current_span()),
                    ));
                }
            }

            let params = self.parse_function_params()?;

            // Parse throws clause: throws(ErrorType | OtherError)
            let throws_clause = self.parse_throws_clause()?;

            // Use parse_type_with_lookahead to support refinements but avoid consuming body {
            let return_type = if self.stream.consume(&TokenKind::RArrow).is_some() {
                Maybe::Some(self.parse_type_with_lookahead()?)
            } else {
                Maybe::None
            };

            // Parse context requirements for protocol methods
            let contexts = if self.stream.check(&TokenKind::LBracket) {
                self.parse_context_requirements()?
            } else if self.stream.consume(&TokenKind::Using).is_some() {
                self.parse_using_contexts()?
            } else {
                Vec::new()
            };

            // Parse where clause (if present) for protocol methods with constraints
            // Supports both: where type T: Protocol and where T: Protocol
            let (generic_where, meta_where) = if self.stream.check(&TokenKind::Where) {
                let where_clause = self.parse_where_clause()?;
                // Separate type predicates from meta predicates
                let mut generic_predicates = List::new();
                let mut meta_predicates = List::new();
                for pred in where_clause.predicates {
                    match pred.kind {
                        verum_ast::ty::WherePredicateKind::Meta { .. } => {
                            meta_predicates.push(pred);
                        }
                        _ => {
                            generic_predicates.push(pred);
                        }
                    }
                }
                let generic_wc = if !generic_predicates.is_empty() {
                    Maybe::Some(verum_ast::ty::WhereClause {
                        predicates: generic_predicates,
                        span: where_clause.span,
                    })
                } else {
                    Maybe::None
                };
                let meta_wc = if !meta_predicates.is_empty() {
                    Maybe::Some(verum_ast::ty::WhereClause {
                        predicates: meta_predicates,
                        span: where_clause.span,
                    })
                } else {
                    Maybe::None
                };
                (generic_wc, meta_wc)
            } else {
                (Maybe::None, Maybe::None)
            };

            // Contract clauses for protocol methods: requires EXPR, ensures EXPR (repeatable)
            // This allows protocol methods to specify contracts that implementations must satisfy
            let mut requires = Vec::new();
            let mut ensures = Vec::new();

            loop {
                // Safety: prevent infinite loop
                if !self.tick() || self.is_aborted() {
                    break;
                }

                match self.stream.peek_kind() {
                    Some(TokenKind::Requires) => {
                        self.stream.advance();
                        let expr = self.parse_expr_no_struct()?;
                        requires.push(expr);
                    }
                    Some(TokenKind::Ensures) => {
                        self.stream.advance();
                        let expr = self.parse_expr_no_struct()?;
                        ensures.push(expr);
                    }
                    // Handle `where ensures EXPR` postcondition syntax
                    Some(TokenKind::Where) if self.stream.peek_nth(1).map(|t| &t.kind) == Some(&TokenKind::Ensures) => {
                        self.stream.advance(); // consume 'where'
                        // Parse one or more ensures items (separated by comma)
                        loop {
                            if self.stream.consume(&TokenKind::Ensures).is_some() {
                                let expr = self.parse_expr_no_struct()?;
                                ensures.push(expr);
                                // Check for comma to continue
                                if self.stream.consume(&TokenKind::Comma).is_none() {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                    }
                    Some(TokenKind::ContractLiteral(_)) => {
                        let expr = self.parse_expr_no_struct()?;
                        requires.push(expr);
                    }
                    _ => break,
                }
            }

            // Default implementation
            let default_impl = if self.stream.check(&TokenKind::LBrace) {
                Maybe::Some(FunctionBody::Block(self.parse_block()?))
            } else if self.stream.consume(&TokenKind::Eq).is_some() {
                let expr = self.parse_expr()?;
                self.stream.expect(TokenKind::Semicolon)?;
                Maybe::Some(FunctionBody::Expr(expr))
            } else {
                // Protocol method signature without body - semicolon is optional
                // Both `fn foo() -> Int;` and `fn foo() -> Int` are valid
                self.stream.consume(&TokenKind::Semicolon);
                Maybe::None
            };

            let span = self.stream.make_span(start_pos);
            let decl = FunctionDecl {
                visibility: Visibility::Private,
                is_async,
                is_meta,
                stage_level,
                is_pure,
                is_generator,
                is_cofix: false,
                is_unsafe,
                is_transparent: false,  // Protocol methods cannot be transparent
                extern_abi: Maybe::None,
                is_variadic: false,
                name: Ident::new(name, name_span),
                generics: generics.clone(),
                params: params.into_iter().collect(),
                return_type,
                throws_clause,
                std_attr: None,
                contexts: contexts.into_iter().collect(),
                generic_where_clause: generic_where,
                meta_where_clause: meta_where,
                requires: requires.into_iter().collect(),
                ensures: ensures.into_iter().collect(),
                attributes: List::new(),
                body: None,
                span,
            };

            return Ok(ProtocolItem {
                kind: ProtocolItemKind::Function { decl, default_impl },
                span,
            });
        }

        Err(ParseError::invalid_syntax(
            "expected protocol item (type, const, fn, meta fn, pure fn, async fn, or unsafe fn)",
            self.stream.current_span(),
        ))
    }

    /// Parse a standalone protocol declaration.
    /// Syntax: [context] protocol Name<T> [extends Base1 + Base2] { ... }
    ///
    /// # Arguments
    /// * `attrs` - Attributes applied to the protocol
    /// * `vis` - Visibility modifier
    /// * `is_context` - Whether this is a context protocol (`context protocol Name`)
    fn parse_protocol_decl_with_context(
        &mut self,
        attrs: Vec<Attribute>,
        vis: Visibility,
        is_context: bool,
    ) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // protocol keyword
        self.stream.expect(TokenKind::Protocol)?;

        // Protocol name
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        // Generic parameters: <F<_>, T>
        let generics: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?
        } else {
            List::new()
        };

        // Extends clause: extends Base1 + Base2 OR : Base1 + Base2
        // Both `extends` keyword and `:` shorthand are accepted for protocol bounds
        let bounds: List<Type> = if self.stream.consume(&TokenKind::Extends).is_some()
            || self.stream.consume(&TokenKind::Colon).is_some()
        {
            self.parse_protocol_extends()?.into_iter().collect()
        } else {
            List::new()
        };

        // Where clauses: where type T: Ord or where meta N > 0
        let (generic_where, meta_where) = if self.stream.peek_kind() == Some(&TokenKind::Where) {
            let where_clause = self.parse_where_clause()?;

            // Separate generic and meta predicates
            let mut generic_preds = Vec::new();
            let mut meta_preds = Vec::new();

            for pred in where_clause.predicates.iter() {
                match &pred.kind {
                    WherePredicateKind::Meta { .. } => meta_preds.push(pred.clone()),
                    _ => generic_preds.push(pred.clone()),
                }
            }

            let generic_clause = if !generic_preds.is_empty() {
                Maybe::Some(WhereClause {
                    predicates: generic_preds.into_iter().collect::<List<_>>(),
                    span: where_clause.span,
                })
            } else {
                Maybe::None
            };

            let meta_clause = if !meta_preds.is_empty() {
                Maybe::Some(WhereClause {
                    predicates: meta_preds.into_iter().collect::<List<_>>(),
                    span: where_clause.span,
                })
            } else {
                Maybe::None
            };

            (generic_clause, meta_clause)
        } else {
            (Maybe::None, Maybe::None)
        };

        // E051: Missing protocol brace - check if 'fn' or 'type' instead of '{'
        if self.stream.check(&TokenKind::Fn) || self.stream.check(&TokenKind::Type) {
            return Err(ParseError::missing_protocol_brace(self.stream.current_span()));
        }

        // Protocol items
        self.stream.expect(TokenKind::LBrace)?;
        let items = self.parse_protocol_items()?;
        self.stream.expect(TokenKind::RBrace)?;

        let span = self.stream.make_span(start_pos);
        Ok(Item::new(
            ItemKind::Protocol(ProtocolDecl {
                visibility: vis,
                is_context,
                name: Ident::new(name, name_span),
                generics,
                bounds,
                items: items.into_iter().collect(),
                generic_where_clause: generic_where,
                meta_where_clause: meta_where,
                span,
            }),
            span,
        ))
    }

    /// Parse an implementation block.
    /// If `is_unsafe` is true, the 'unsafe' keyword has already been consumed.
    fn parse_impl_with_unsafe(&mut self, attrs: Vec<Attribute>, is_unsafe: bool) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // Validate impl attributes
        self.validate_attrs_for_target(&attrs, AttributeTarget::Impl);

        self.stream.expect(TokenKind::Implement)?;

        // Handle negative impls: `implement !Send for Type { }`
        let is_negative = self.stream.consume(&TokenKind::Bang).is_some();

        // Generic parameters
        let generics: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?
        } else {
            List::new()
        };

        // E054: Missing impl type - check if '{' or 'for' comes immediately
        match self.stream.peek_kind() {
            Some(TokenKind::LBrace) => {
                if !is_negative {
                    return Err(ParseError::missing_impl_type(self.stream.current_span()));
                }
            }
            Some(TokenKind::For) => {
                if !is_negative {
                    return Err(ParseError::missing_impl_type(self.stream.current_span()));
                }
            }
            _ => {}
        }

        // Protocol for Type or just Type
        let kind = self.parse_impl_kind()?;

        // Where clause: where T: Ord or where type T: Ord or where meta N > 0
        let (generic_where, meta_where) = if self.stream.peek_kind() == Some(&TokenKind::Where) {
            // Parse the where clause - it handles both generic and meta predicates
            let where_clause = self.parse_where_clause()?;

            // Separate generic and meta predicates
            let mut generic_preds = Vec::new();
            let mut meta_preds = Vec::new();

            for pred in where_clause.predicates.iter() {
                match &pred.kind {
                    WherePredicateKind::Meta { .. } => meta_preds.push(pred.clone()),
                    _ => generic_preds.push(pred.clone()),
                }
            }

            let generic_clause = if !generic_preds.is_empty() {
                Maybe::Some(WhereClause {
                    predicates: generic_preds.into_iter().collect::<List<_>>(),
                    span: where_clause.span,
                })
            } else {
                Maybe::None
            };

            let meta_clause = if !meta_preds.is_empty() {
                Maybe::Some(WhereClause {
                    predicates: meta_preds.into_iter().collect::<List<_>>(),
                    span: where_clause.span,
                })
            } else {
                Maybe::None
            };

            (generic_clause, meta_clause)
        } else {
            (Maybe::None, Maybe::None)
        };

        // Items
        // E057: Missing impl brace
        if !self.stream.check(&TokenKind::LBrace) {
            return Err(ParseError::missing_impl_brace(self.stream.current_span()));
        }
        self.stream.advance(); // consume '{'
        let items = self.parse_impl_items()?;
        self.stream.expect(TokenKind::RBrace)?;

        // Extract @specialize attribute
        let specialize_attr = self.parse_specialize_from_attrs(&attrs);

        let span = self.stream.make_span(start_pos);
        Ok(Item::new_with_attrs(
            ItemKind::Impl(ImplDecl {
                is_unsafe,
                generics,
                kind,
                generic_where_clause: generic_where,
                meta_where_clause: meta_where,
                specialize_attr,
                items: items.into_iter().collect(),
                span,
            }),
            attrs.into_iter().collect(),
            span,
        ))
    }

    /// Parse impl kind: Protocol for Type or just Type
    fn parse_impl_kind(&mut self) -> ParseResult<ImplKind> {
        // Parse the first type (could be Protocol<T> or Type<T>)
        // Use parse_type_no_refinement because the { } after is the impl block, not a refinement
        let first_type = self.parse_type_no_refinement()?;

        // E055: Check for missing 'for' keyword
        // If next token is an identifier (looks like a type), 'for' is likely missing
        if let Some(TokenKind::Ident(_)) = self.stream.peek_kind() {
            return Err(ParseError::missing_impl_for(self.stream.current_span()));
        }

        if self.stream.consume(&TokenKind::For).is_some() {
            // It's a protocol implementation: Protocol<T> for Type<U>
            // The protocol can be either:
            // - A simple path: Functor
            // - A generic type with HKT: Functor<List> where List is a bare type constructor
            // For HKT support, we preserve the type constructor arguments in protocol_args

            // Parse the type being implemented for
            let for_type = self.parse_type_no_refinement()?;

            // Extract protocol path and type constructor arguments
            let (protocol, protocol_args) = match first_type.kind {
                TypeKind::Path(path) => {
                    // Simple protocol without type arguments: Functor
                    (path, List::new())
                }
                TypeKind::Generic { base, args } => {
                    // Protocol with HKT arguments: Functor<List>
                    // Extract the base path and preserve the type constructor arguments
                    match base.kind {
                        TypeKind::Path(path) => (path, args),
                        _ => {
                            return Err(ParseError::invalid_syntax(
                                "expected protocol path before 'for'",
                                first_type.span,
                            ));
                        }
                    }
                }
                // Handle associated type as qualified path: super.Protocol, module.Protocol
                TypeKind::AssociatedType { base, assoc } => {
                    // Convert associated type to a multi-segment path
                    let mut segments: Vec<PathSegment> = Vec::new();
                    // Extract segments from base type
                    fn extract_path_segments(ty: &Type, segments: &mut Vec<PathSegment>) {
                        match &ty.kind {
                            TypeKind::Path(path) => {
                                segments.extend(path.segments.iter().cloned());
                            }
                            TypeKind::AssociatedType { base, assoc } => {
                                extract_path_segments(base, segments);
                                segments.push(PathSegment::Name(assoc.clone()));
                            }
                            _ => {}
                        }
                    }
                    extract_path_segments(&base, &mut segments);
                    segments.push(PathSegment::Name(assoc));
                    let path = Path::new(
                        segments.into_iter().collect::<List<_>>(),
                        first_type.span,
                    );
                    (path, List::new())
                }
                _ => {
                    return Err(ParseError::invalid_syntax(
                        "expected protocol path before 'for'",
                        first_type.span,
                    ));
                }
            };

            Ok(ImplKind::Protocol {
                protocol,
                protocol_args,
                for_type,
            })
        } else {
            // It's an inherent implementation: Type<T>
            Ok(ImplKind::Inherent(first_type))
        }
    }

    /// Parse implementation items.
    ///
    /// SAFETY: Includes forward progress check to prevent infinite loops
    /// on malformed input.
    fn parse_impl_items(&mut self) -> ParseResult<Vec<ImplItem>> {
        let mut items = Vec::new();

        while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }

            let pos_before = self.stream.position();

            match self.parse_impl_item() {
                Ok(item) => items.push(item),
                Err(e) => {
                    self.error(e);
                    self.synchronize();
                }
            }

            // CRITICAL FIX: Ensure forward progress to prevent infinite loop
            if self.stream.position() == pos_before && !self.stream.at_end() {
                self.stream.advance();
            }
        }

        Ok(items)
    }

    /// Parse a single impl item.
    fn parse_impl_item(&mut self) -> ParseResult<ImplItem> {
        let start_pos = self.stream.position();

        // Parse attributes first (e.g., @inject for DI)
        let attributes = self.parse_attributes()?;

        let vis = self.parse_visibility()?;

        // Skip 'default' contextual keyword (used for specialization: `default fn ...`)
        // 'default' is lexed as an identifier, not a keyword
        if let Some(Token { kind: TokenKind::Ident(name), .. }) = self.stream.peek() {
            if name.as_str() == "default" {
                // Check if followed by fn, type, const, async, unsafe, meta, pure
                let next = self.stream.peek_nth_kind(1);
                if matches!(next,
                    Some(&TokenKind::Fn) | Some(&TokenKind::Type) | Some(&TokenKind::Const)
                    | Some(&TokenKind::Async) | Some(&TokenKind::Unsafe)
                    | Some(&TokenKind::Meta) | Some(&TokenKind::Pure)
                ) {
                    self.stream.advance(); // consume 'default'
                }
            }
        }

        // Proof clause — `proof axiom_name by tactic;` (T1-R phase 2).
        //
        // Inside an `implement P for T { … }` block, this discharges
        // the named axiom from protocol P using the given tactic. The
        // model-verification phase matches the name against P's axiom
        // list and runs the tactic against the self-substituted
        // proposition instead of the default auto_prove.
        if self.stream.check(&TokenKind::Proof) {
            self.stream.advance();
            let axiom_name_str = self.consume_ident_or_any_keyword()?;
            let axiom_name_span = self.stream.current_span();
            self.stream.expect(TokenKind::By)?;
            // Use parse_tactic_primary (single tactic) rather than
            // parse_tactic_expr (sequence) so that `;` terminates the
            // proof-clause — otherwise a subsequent `proof X by …;`
            // would be greedily absorbed as a continuation.
            let tactic = self.parse_tactic_primary()?;
            self.stream.consume(&TokenKind::Semicolon);
            let span = self.stream.make_span(start_pos);
            return Ok(ImplItem {
                attributes: attributes.into_iter().collect(),
                visibility: vis,
                kind: ImplItemKind::Proof {
                    axiom_name: Ident::new(axiom_name_str, axiom_name_span),
                    tactic,
                },
                span,
            });
        }

        // Type alias
        if self.stream.check(&TokenKind::Type) {
            self.stream.advance();
            // Use consume_ident_or_any_keyword to allow keywords like Err, Error as type names
            let name = self.consume_ident_or_any_keyword()?;
            let name_span = self.stream.current_span();

            // Generic parameters for GATs: type F<T>
            // GATs: associated types with own type params, e.g., `type Item<T> = List<T>;`
            // Example: `type Item<T> = List<T>;` in impl blocks
            let type_params = if self.stream.check(&TokenKind::Lt) {
                self.parse_generic_params()?
            } else {
                List::new()
            };

            // Protocol item: `type Name;` (declaration only, no value)
            if self.stream.check(&TokenKind::Semicolon) {
                self.stream.advance();
                let span = self.stream.make_span(start_pos);
                return Ok(ImplItem {
                    attributes: attributes.into_iter().collect(),
                    visibility: vis,
                    kind: ImplItemKind::Type {
                        name: Ident::new(name, name_span),
                        type_params,
                        ty: verum_ast::ty::Type::new(
                            verum_ast::ty::TypeKind::Unit,
                            span,
                        ),
                    },
                    span,
                });
            }

            // Accept either = or is for associated type definition
            if self.stream.consume(&TokenKind::Eq).is_none() {
                self.stream.expect(TokenKind::Is)?;
            }
            let ty = self.parse_type()?;
            self.stream.expect(TokenKind::Semicolon)?;

            let span = self.stream.make_span(start_pos);
            return Ok(ImplItem {
                attributes: attributes.into_iter().collect(),
                visibility: vis,
                kind: ImplItemKind::Type {
                    name: Ident::new(name, name_span),
                    type_params,
                    ty,
                },
                span,
            });
        }

        // Const value (const NAME: TYPE = VALUE;)
        if self.stream.check(&TokenKind::Const) {
            self.stream.advance();
            let name = self.consume_ident()?;
            let name_span = self.stream.current_span();

            self.stream.expect(TokenKind::Colon)?;
            let ty = self.parse_type()?;

            self.stream.expect(TokenKind::Eq)?;
            let value = self.parse_expr()?;

            self.stream.expect(TokenKind::Semicolon)?;

            let span = self.stream.make_span(start_pos);
            return Ok(ImplItem {
                attributes: attributes.into_iter().collect(),
                visibility: vis,
                kind: ImplItemKind::Const {
                    name: Ident::new(name, name_span),
                    ty,
                    value,
                },
                span,
            });
        }

        // Function (with optional pure, meta, unsafe, async and optional generator *)
        // Grammar: function_modifiers = [ 'pure' ] , [ 'meta' ] , [ 'async' ] , [ 'unsafe' ] | epsilon
        // Grammar: fn_keyword = 'fn' , [ '*' ] for generator functions
        // Order: [pure] [meta] [unsafe] [async] fn [*] name
        if self.stream.check(&TokenKind::Fn)
            || self.stream.check(&TokenKind::Async)
            || self.stream.check(&TokenKind::Unsafe)
            || self.stream.check(&TokenKind::Meta)
            || self.stream.check(&TokenKind::Pure)
        {
            let is_pure = self.stream.consume(&TokenKind::Pure).is_some();
            // Parse meta with optional stage level: meta or meta(N)
            let (is_meta, stage_level) = self.parse_meta_modifier()?;
            let is_unsafe = self.stream.consume(&TokenKind::Unsafe).is_some();
            let is_async = self.stream.consume(&TokenKind::Async).is_some();
            self.stream.expect(TokenKind::Fn)?;

            // Check for generator syntax: fn* or async fn*
            let is_generator = self.stream.consume(&TokenKind::Star).is_some();

            // E056: Missing function name - `fn;` or `fn(`
            if self.stream.check(&TokenKind::Semicolon)
                || self.stream.check(&TokenKind::LParen)
                || self.stream.check(&TokenKind::RBrace)
            {
                return Err(ParseError::invalid_impl_method(
                    "missing function name after 'fn'",
                    self.stream.current_span(),
                ));
            }

            // E056: Literal instead of name - `fn 123`
            if let Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_)) | Some(TokenKind::Text(_)) =
                self.stream.peek_kind()
            {
                return Err(ParseError::invalid_impl_method(
                    "expected function name, found literal",
                    self.stream.current_span(),
                ));
            }

            // Allow contextual keywords as function names (e.g., `show` which is a proof keyword)
            let name = self.consume_ident_or_keyword()?;
            let name_span = self.stream.current_span();

            // E056: Missing parentheses - `fn name 123`
            if !self.stream.check(&TokenKind::LParen) && !self.stream.check(&TokenKind::Lt) {
                if let Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_)) | Some(TokenKind::Text(_)) =
                    self.stream.peek_kind()
                {
                    return Err(ParseError::invalid_impl_method(
                        "expected '(' after function name, found literal",
                        self.stream.current_span(),
                    ));
                }
            }

            let generics: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
                self.parse_generic_params()?
            } else {
                List::new()
            };
            let params = self.parse_function_params()?;

            // Parse throws clause: throws(ErrorType | OtherError)
            let throws_clause = self.parse_throws_clause()?;

            // Use parse_type_with_lookahead to support refinements but avoid consuming body {
            let return_type = if self.stream.consume(&TokenKind::RArrow).is_some() {
                Maybe::Some(self.parse_type_with_lookahead()?)
            } else {
                Maybe::None
            };

            // Context requirements (canonical syntax per grammar: after return type)
            // GRAMMAR: fn foo() -> Int using [IO] or fn foo() -> Int [IO]
            let contexts = if self.stream.check(&TokenKind::LBracket) {
                self.parse_context_requirements()?
            } else if self.stream.consume(&TokenKind::Using).is_some() {
                self.parse_using_contexts()?
            } else {
                Vec::new()
            };

            // Parse where clause (if present)
            // Supports both: where type T: Protocol and where T: Protocol
            let (generic_where, meta_where) = if self.stream.check(&TokenKind::Where) {
                let where_clause = self.parse_where_clause()?;
                // Separate type predicates from meta predicates
                let mut generic_predicates = List::new();
                let mut meta_predicates = List::new();
                for pred in where_clause.predicates {
                    match pred.kind {
                        verum_ast::ty::WherePredicateKind::Meta { .. } => {
                            meta_predicates.push(pred);
                        }
                        _ => {
                            generic_predicates.push(pred);
                        }
                    }
                }
                let generic_wc = if !generic_predicates.is_empty() {
                    Maybe::Some(verum_ast::ty::WhereClause {
                        predicates: generic_predicates,
                        span: where_clause.span,
                    })
                } else {
                    Maybe::None
                };
                let meta_wc = if !meta_predicates.is_empty() {
                    Maybe::Some(verum_ast::ty::WhereClause {
                        predicates: meta_predicates,
                        span: where_clause.span,
                    })
                } else {
                    Maybe::None
                };
                (generic_wc, meta_wc)
            } else {
                (Maybe::None, Maybe::None)
            };

            // Contract clauses: requires EXPR, ensures EXPR (repeatable)
            let mut requires = Vec::new();
            let mut ensures = Vec::new();
            loop {
                if !self.tick() || self.is_aborted() {
                    break;
                }
                match self.stream.peek_kind() {
                    Some(TokenKind::Requires) => {
                        self.stream.advance();
                        let expr = self.parse_expr_no_struct()?;
                        requires.push(expr);
                    }
                    Some(TokenKind::Ensures) => {
                        self.stream.advance();
                        let expr = self.parse_expr_no_struct()?;
                        ensures.push(expr);
                    }
                    _ => break,
                }
            }

            // Parse function body or semicolon for bodiless functions (intrinsics)
            let body = if self.stream.check(&TokenKind::LBrace) {
                Maybe::Some(FunctionBody::Block(self.parse_block()?))
            } else if self.stream.consume(&TokenKind::Eq).is_some() {
                let expr = self.parse_expr()?;
                self.stream.expect(TokenKind::Semicolon)?;
                Maybe::Some(FunctionBody::Expr(expr))
            } else if self.stream.check(&TokenKind::Semicolon) {
                // Bodiless function (e.g., @compiler_intrinsic pub fn evaluate() -> Bool;)
                self.stream.advance();
                Maybe::None
            } else {
                // E056: Impl method requires body or semicolon
                return Err(ParseError::invalid_impl_method(
                    "expected '{' or ';' after impl method signature",
                    self.stream.current_span(),
                ));
            };

            // Check for @transparent attribute in impl item
            let is_transparent = attributes.iter().any(|a| a.name.as_str() == "transparent");

            let span = self.stream.make_span(start_pos);
            let decl = FunctionDecl {
                visibility: vis.clone(),
                is_async,
                is_meta,
                stage_level,
                is_pure,
                is_generator,
                is_cofix: false,
                is_unsafe,
                is_transparent,
                extern_abi: Maybe::None,
                is_variadic: false,
                name: Ident::new(name, name_span),
                generics,
                params: params.into_iter().collect(),
                return_type,
                throws_clause,
                std_attr: None,
                contexts: contexts.into_iter().collect(),
                generic_where_clause: generic_where,
                meta_where_clause: meta_where,
                requires: requires.into_iter().collect(),
                ensures: ensures.into_iter().collect(),
                attributes: attributes.clone().into(),
                body,
                span,
            };

            return Ok(ImplItem {
                attributes: attributes.into_iter().collect(),
                visibility: vis,
                kind: ImplItemKind::Function(decl),
                span,
            });
        }

        Err(ParseError::invalid_syntax(
            "expected impl item (fn, meta fn, type, or const)",
            self.stream.current_span(),
        ))
    }

    /// Parse a const declaration.
    fn parse_const(&mut self, vis: Visibility) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // Accept both `const` and `let` at item level
        if self.stream.consume(&TokenKind::Const).is_none() {
            self.stream.expect(TokenKind::Let)?;
        }
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        // Optional generic parameters: const ZERO<T: Default>: T = T.default();
        let generics = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?.into_iter().collect()
        } else {
            List::new()
        };

        // Type annotation for const declarations:
        // `const FOO: Int = 42;` — explicit type
        // `const FOO = 42;`      — inferred type (type-checker resolves)
        // Both forms are accepted at any scope level. The type-checker
        // infers the type from the initialiser when the annotation is
        // omitted — this matches the test expectation
        // `test_const_inferred_type` and the general Verum philosophy
        // of minimal ceremony for simple declarations.
        let ty = if self.stream.check(&TokenKind::Colon) {
            self.stream.advance(); // consume ':'
            self.parse_type()?
        } else {
            verum_ast::ty::Type::unknown(self.stream.current_span())
        };

        // E066: Missing const value - check if ';' instead of '='
        if self.stream.check(&TokenKind::Semicolon) {
            return Err(ParseError::missing_const_value(self.stream.current_span()));
        }

        self.stream.expect(TokenKind::Eq)?;

        // E066: Missing const value - check if ';' immediately after '='
        if self.stream.check(&TokenKind::Semicolon) {
            return Err(ParseError::missing_const_value(self.stream.current_span()));
        }

        let value = self.parse_expr()?;

        self.stream.expect(TokenKind::Semicolon)?;

        let span = self.stream.make_span(start_pos);
        Ok(Item::new(
            ItemKind::Const(ConstDecl {
                visibility: vis,
                name: Ident::new(name, name_span),
                generics,
                ty,
                value,
                span,
            }),
            span,
        ))
    }

    /// Parse a static declaration.
    fn parse_static(&mut self, attrs: Vec<Attribute>, vis: Visibility) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        self.stream.expect(TokenKind::Static)?;
        let is_mut = self.stream.consume(&TokenKind::Mut).is_some();

        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        // E067: Missing static type annotation - check if '=' instead of ':'
        if self.stream.check(&TokenKind::Eq) {
            return Err(ParseError::missing_static_type(self.stream.current_span()));
        }

        self.stream.expect(TokenKind::Colon)?;
        let ty = self.parse_type()?;

        // Value is optional for extern/FFI statics
        let value = if self.stream.consume(&TokenKind::Eq).is_some() {
            self.parse_expr()?
        } else {
            // Default to unit expression for uninitialized statics (e.g., FFI/extern statics)
            Expr::new(ExprKind::Tuple(List::new()), self.stream.current_span())
        };

        self.stream.expect(TokenKind::Semicolon)?;

        let span = self.stream.make_span(start_pos);
        Ok(Item::new_with_attrs(
            ItemKind::Static(StaticDecl {
                visibility: vis,
                is_mut,
                name: Ident::new(name, name_span),
                ty,
                value,
                span,
            }),
            attrs.into(),
            span,
        ))
    }

    /// Parse a module declaration.
    fn parse_module_decl(&mut self, attrs: Vec<Attribute>, vis: Visibility) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // Validate module attributes
        self.validate_attrs_for_target(&attrs, AttributeTarget::Module);

        self.stream.expect(TokenKind::Module)?;

        // E061: Missing module name - check if '{' comes immediately
        if self.stream.check(&TokenKind::LBrace) {
            return Err(ParseError::missing_module_name(self.stream.current_span()));
        }

        // Parse module name with optional dot-separated path (e.g., `module runtime.recovery;`)
        // This allows hierarchical module naming like Python packages
        let first_name = self.consume_ident_or_any_keyword()?;
        let mut full_name = first_name.to_string();
        let name_start = self.stream.current_span();

        // Parse optional dot-separated path segments
        while self.stream.check(&TokenKind::Dot) {
            self.stream.advance(); // consume '.'
            let segment = self.consume_ident_or_any_keyword()?;
            full_name.push('.');
            full_name.push_str(segment.as_str());
        }

        let name = Text::from(full_name);
        let name_span = name_start;

        // Module can be forward declaration (;) or have a body ({...})
        // E062: Missing module brace - check if neither '{' nor ';' is present
        if !self.stream.check(&TokenKind::LBrace) && !self.stream.check(&TokenKind::Semicolon) {
            return Err(ParseError::missing_module_brace(self.stream.current_span()));
        }
        let items = if self.stream.check(&TokenKind::LBrace) {
            self.stream.advance();
            let mut module_items = Vec::new();

            while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
                // Safety: prevent infinite loop
                if !self.tick() || self.is_aborted() {
                    break;
                }

                let pos_before = self.stream.position();
                match self.parse_item() {
                    Ok(item) => module_items.push(item),
                    Err(e) => {
                        self.error(e);
                        self.synchronize();
                        // Prevent infinite loop
                        if self.stream.position() == pos_before && !self.stream.at_end() {
                            self.stream.advance();
                        }
                        if self.stream.at_end() {
                            break;
                        }
                    }
                }
            }

            self.stream.expect(TokenKind::RBrace)?;
            Maybe::Some(module_items)
        } else {
            self.stream.expect(TokenKind::Semicolon)?;
            Maybe::None
        };

        // Extract @profile, @features, and @using attributes
        let mut profile_attr: Maybe<verum_ast::attr::ProfileAttr> = None;
        let mut feature_attr: Maybe<verum_ast::attr::FeatureAttr> = None;
        let mut context_requirements: Vec<ContextRequirement> = Vec::new();

        for attr in attrs.iter() {
            if attr.name.as_str() == "profile" {
                if let Some(args) = &attr.args {
                    let mut profiles = Vec::new();
                    for arg in args.iter() {
                        if let verum_ast::ExprKind::Path(path) = &arg.kind
                            && let Some(seg) = path.segments.first()
                            && let verum_ast::PathSegment::Name(ident) = seg
                        {
                            let profile = match ident.name.as_str() {
                                "application" => verum_ast::attr::Profile::Application,
                                "systems" => verum_ast::attr::Profile::Systems,
                                "research" => verum_ast::attr::Profile::Research,
                                _ => verum_ast::attr::Profile::Application,
                            };
                            profiles.push(profile);
                        }
                    }
                    profile_attr = Some(verum_ast::attr::ProfileAttr::new(profiles.into(), attr.span));
                }
            } else if attr.name.as_str() == "features" {
                if let Some(args) = &attr.args {
                    let mut features = Vec::new();
                    for arg in args.iter() {
                        if let verum_ast::ExprKind::Path(path) = &arg.kind
                            && let Some(seg) = path.segments.first()
                            && let verum_ast::PathSegment::Name(ident) = seg
                        {
                            features.push(ident.name.clone());
                        }
                    }
                    feature_attr = Some(verum_ast::attr::FeatureAttr::new(features.into(), attr.span));
                }
            } else if attr.name.as_str() == "using" {
                // Parse @using attribute: @using([Database, Logger]) or @using(Database)
                if let Some(args) = &attr.args {
                    for arg in args.iter() {
                        // Handle both array of contexts: [Database, Logger]
                        // and single context: Database
                        match &arg.kind {
                            verum_ast::ExprKind::Array(verum_ast::expr::ArrayExpr::List(
                                contexts,
                            )) => {
                                for ctx_expr in contexts.iter() {
                                    if let Some(path) = Self::expr_to_path(ctx_expr) {
                                        context_requirements.push(ContextRequirement::simple(
                                            path,
                                            List::new(),
                                            ctx_expr.span,
                                        ));
                                    }
                                }
                            }
                            _ => {
                                // Try to convert any expression to a path (handles Path, Field access, etc.)
                                if let Some(path) = Self::expr_to_path(arg) {
                                    context_requirements.push(ContextRequirement::simple(
                                        path,
                                        List::new(),
                                        arg.span,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        let span = self.stream.make_span(start_pos);
        Ok(Item::new(
            ItemKind::Module(ModuleDecl {
                visibility: vis,
                name: Ident::new(name, name_span),
                items: items.map(|v| v.into()),
                profile: profile_attr,
                features: feature_attr,
                contexts: context_requirements.into_iter().collect(),
                span,
            }),
            span,
        ))
    }

    /// Parse a mount declaration.
    ///
    /// Mount statements can have @cfg attributes for conditional compilation:
    /// ```verum
    /// @cfg(target_os = "linux")
    /// mount super.sys.linux.syscall.{write as sys_write};
    /// ```
    fn parse_mount(&mut self, attrs: List<Attribute>, vis: Visibility) -> ParseResult<Item> {
        let start_pos = self.stream.position();
        let is_public = matches!(vis, Visibility::Public);

        // Accept both `mount` and `link` keywords
        if self.stream.consume(&TokenKind::Mount).is_none() {
            self.stream.expect(TokenKind::Link)?;
        }

        // E063/E064: Missing path - `mount;` or `public mount;`
        if self.stream.check(&TokenKind::Semicolon) {
            return Err(if is_public {
                ParseError::invalid_pub_use_syntax(
                    "expected module path after 'mount'",
                    self.stream.current_span(),
                )
            } else {
                ParseError::invalid_mount_syntax(
                    "expected module path after 'mount'",
                    self.stream.current_span(),
                )
            });
        }

        // E063/E064: Literal instead of path - `mount 123;` or `public mount 123;`
        if let Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_)) | Some(TokenKind::Text(_)) =
            self.stream.peek_kind()
        {
            return Err(if is_public {
                ParseError::invalid_pub_use_syntax(
                    "expected module path, found literal",
                    self.stream.current_span(),
                )
            } else {
                ParseError::invalid_mount_syntax(
                    "expected module path, found literal",
                    self.stream.current_span(),
                )
            });
        }

        let tree = self.parse_mount_tree()?;

        let alias = if self.stream.consume(&TokenKind::As).is_some() {
            // E063: Missing alias - `mount std.* as;`
            if self.stream.check(&TokenKind::Semicolon) {
                return Err(ParseError::invalid_mount_syntax(
                    "expected alias name after 'as'",
                    self.stream.current_span(),
                ));
            }
            // Allow keywords like Ok, Err, Some, None as alias names
            // These are valid identifiers in mount alias context
            let name = self.consume_ident_or_keyword()?;
            let name_span = self.stream.current_span();
            Maybe::Some(Ident::new(name, name_span))
        } else {
            Maybe::None
        };

        // Semicolon is optional if followed by statement/block terminators
        if !self.allows_semicolon_omission() {
            self.stream.expect(TokenKind::Semicolon)?;
        } else {
            self.stream.consume(&TokenKind::Semicolon);
        }

        let span = self.stream.make_span(start_pos);
        Ok(Item::new_with_attrs(
            ItemKind::Mount(MountDecl {
                visibility: vis,
                tree,
                alias,
                span,
            }),
            attrs,
            span,
        ))
    }

    /// Parse a mount tree.
    ///
    /// Recursion depth is checked to prevent stack overflow from
    /// deeply nested mount groups like `mount a.{b.{c.{d.{...}}}}`.
    fn parse_mount_tree(&mut self) -> ParseResult<MountTree> {
        self.enter_recursion()?;
        let result = self.parse_mount_tree_inner();
        self.exit_recursion();
        result
    }

    /// Inner implementation of parse_mount_tree (after recursion check).
    fn parse_mount_tree_inner(&mut self) -> ParseResult<MountTree> {
        let start_pos = self.stream.position();

        // #5 / P1.5 — detect file-relative mount before
        // falling into module-path parsing.  `./foo.vr` and
        // `../shared/util.vr` start with `Dot Slash` or
        // `Dot Dot Slash`, neither of which can begin a
        // module path (module-path's leading dot is followed
        // by an identifier, not `/`).
        if self.is_file_mount_lookahead() {
            return self.parse_file_mount_tree(start_pos);
        }

        // Parse the prefix path (at least one segment)
        let mut segments = Vec::new();

        // Check for optional leading dot (relative mount)
        if self.stream.consume(&TokenKind::Dot).is_some() {
            segments.push(PathSegment::Relative);
        }

        segments.push(self.parse_path_segment()?);

        // Continue parsing path segments until we hit .* or .{
        loop {
            // Check if next is dot
            if !self.stream.check(&TokenKind::Dot) {
                break;
            }

            // Look ahead to see what comes after the dot
            match self.stream.peek_nth(1).map(|t| &t.kind) {
                Some(TokenKind::Star) => {
                    // This is a glob mount: path.*
                    self.stream.consume(&TokenKind::Dot);
                    self.stream.consume(&TokenKind::Star);

                    let span = self.stream.make_span(start_pos);
                    let path = Path::new(segments.into_iter().collect(), span);
                    return Ok(MountTree {
                        kind: MountTreeKind::Glob(path),
                        alias: Maybe::None,
                        span,
                    });
                }
                Some(TokenKind::LBrace) => {
                    // This is a nested mount: path.{a, b, c} or path.{a as b, c as d}
                    self.stream.consume(&TokenKind::Dot);
                    self.stream.advance(); // consume {

                    let mut trees = Vec::new();
                    if !self.stream.check(&TokenKind::RBrace) {
                        loop {
                            let pos_before = self.stream.position();
                            let mut tree = self.parse_mount_tree()?;

                            // Check for 'as' alias within nested mounts
                            if self.stream.consume(&TokenKind::As).is_some() {
                                let alias_name = self.consume_ident_or_keyword()?;
                                let alias_span = self.stream.current_span();
                                tree.alias = Maybe::Some(Ident::new(alias_name, alias_span));
                            }

                            trees.push(tree);

                            // Safety: Ensure we made forward progress
                            if self.stream.position() == pos_before {
                                return Err(ParseError::invalid_syntax(
                                    "parser made no progress in mount list",
                                    self.stream.current_span(),
                                ));
                            }

                            if self.stream.consume(&TokenKind::Comma).is_none() {
                                // E063: Missing comma in nested mount - `{List Map}`
                                if !self.stream.check(&TokenKind::RBrace) {
                                    return Err(ParseError::invalid_mount_syntax(
                                        "expected ',' or '}' in mount group",
                                        self.stream.current_span(),
                                    ));
                                }
                                break;
                            }
                            // Allow trailing comma
                            if self.stream.check(&TokenKind::RBrace) {
                                break;
                            }
                        }
                    }

                    self.stream.expect(TokenKind::RBrace)?;
                    let span = self.stream.make_span(start_pos);
                    let path = Path::new(segments.into_iter().collect(), span);
                    return Ok(MountTree {
                        kind: MountTreeKind::Nested {
                            prefix: path,
                            trees: trees.into_iter().collect(),
                        },
                        alias: Maybe::None,
                        span,
                    });
                }
                Some(TokenKind::Dot) => {
                    // E063: Double dot - `mount std..collections;`
                    return Err(ParseError::invalid_mount_syntax(
                        "unexpected double dot in mount path",
                        self.stream.current_span(),
                    ));
                }
                Some(TokenKind::Semicolon) | Some(TokenKind::As) | None => {
                    // E063: Trailing dot - `mount std.;` or `mount std. as alias;`
                    return Err(ParseError::invalid_mount_syntax(
                        "unexpected trailing dot in mount path",
                        self.stream.current_span(),
                    ));
                }
                _ => {
                    // Normal path segment continues
                    self.stream.consume(&TokenKind::Dot);
                    segments.push(self.parse_path_segment()?);
                }
            }
        }

        // Simple path mount
        let span = self.stream.make_span(start_pos);
        let path = Path::new(segments.into_iter().collect(), span);
        Ok(MountTree {
            kind: MountTreeKind::Path(path),
            alias: Maybe::None,
            span,
        })
    }

    /// `true` when the upcoming tokens form a file-relative
    /// mount (`./...` or `../...`).  Disambiguators:
    ///
    ///   * `Dot Slash`     → `./...`     (one-character dot
    ///                                     followed by `/`)
    ///   * `DotDot Slash`  → `../...`    (lexer emits `..` as
    ///                                     a single `DotDot`
    ///                                     token used elsewhere
    ///                                     for range syntax)
    ///
    /// A normal relative module mount is `Dot Ident`, never
    /// `Dot Slash` / `DotDot Slash`, so the lookahead does
    /// not collide with the existing `mount .config.X;` form.
    fn is_file_mount_lookahead(&self) -> bool {
        let kind0 = self.stream.peek_nth(0).map(|t| &t.kind);
        let kind1 = self.stream.peek_nth(1).map(|t| &t.kind);
        match (kind0, kind1) {
            (Some(TokenKind::Dot), Some(TokenKind::Slash)) => true,
            (Some(TokenKind::DotDot), Some(TokenKind::Slash)) => true,
            _ => false,
        }
    }

    /// Parse a file-relative mount path (`./foo.vr`, `../bar/baz.vr`)
    /// into a `MountTree { kind: File, .. }`.  The path is
    /// reassembled from individual tokens — the lexer doesn't
    /// emit a single string token for these — and validated
    /// against the constraints documented on
    /// `MountTreeKind::File`:
    ///
    ///   * starts with `./` or `../`
    ///   * ends with `.vr`
    ///   * contains no `\\0` / `\\n` / `\\r`
    ///   * does NOT escape the cog root via excessive `..`
    ///
    /// Path-traversal validation happens here (parser layer)
    /// rather than in the loader so a malformed mount becomes
    /// a parse error with a span that points at the literal
    /// path, not a deeper "module not found" diagnostic.
    fn parse_file_mount_tree(
        &mut self,
        start_pos: usize,
    ) -> ParseResult<MountTree> {
        // Parser-time escape check: after the leading `./` /
        // `../` prefix, we track per-segment net depth in the
        // *body*.  `..` segments inside the body that would
        // push depth below zero are rejected (`./a/../../X`
        // collapses to `../X`, which the body parser refuses
        // to emit as the prefix is fixed).  Cog-root-wide
        // escape detection (e.g. starting with too many `../`
        // for the source-file location) lives in the loader,
        // which has filesystem context.
        let mut path = String::new();
        let mut last_was_separator = false;
        let mut segment_buf = String::new();
        // Net depth of the body relative to the directory
        // anchored by the prefix.  Starts at 0; each non-
        // `..` non-`.` segment += 1, each `..` segment -= 1.
        let mut body_depth: i64 = 0;

        // Consume the leading `./` or `../` prefix(es).
        // The lexer emits `..` as a single `DotDot` token
        // (used elsewhere for range syntax) and `.` as `Dot`,
        // so the prefix has exactly one `Dot Slash` (single
        // `./`) or one-or-more `DotDot Slash` chains.
        loop {
            let kind0 = self.stream.peek_nth(0).map(|t| &t.kind);
            let kind1 = self.stream.peek_nth(1).map(|t| &t.kind);
            match (kind0, kind1) {
                (Some(TokenKind::Dot), Some(TokenKind::Slash)) => {
                    self.stream.advance(); // .
                    self.stream.advance(); // /
                    path.push_str("./");
                    last_was_separator = true;
                    break; // `./` is followed by the body
                }
                (Some(TokenKind::DotDot), Some(TokenKind::Slash)) => {
                    self.stream.advance(); // ..
                    self.stream.advance(); // /
                    path.push_str("../");
                    last_was_separator = true;
                    // Allow `../../...` chains — only keep
                    // looping while the next pair is *also*
                    // `DotDot Slash`.  Anything else
                    // (Ident, Dot, …) is the start of the
                    // body, so break out.
                    let next0 = self.stream.peek_nth(0).map(|t| &t.kind);
                    let next1 = self.stream.peek_nth(1).map(|t| &t.kind);
                    if matches!(
                        (next0, next1),
                        (Some(TokenKind::DotDot), Some(TokenKind::Slash))
                    ) {
                        continue;
                    }
                    break;
                }
                _ => {
                    return Err(ParseError::invalid_mount_syntax(
                        "expected `./` or `../` to start a file-relative mount",
                        self.stream.current_span(),
                    ));
                }
            }
        }

        // Body: collect tokens until a terminator.  Each
        // path component is `Ident` separated by `Slash` or
        // `Dot` (for the file extension).  We rebuild the
        // textual form so the AST node stores the literal
        // spelling for diagnostics.
        loop {
            let kind = self.stream.peek_kind().cloned();
            match kind {
                Some(TokenKind::Ident(name)) => {
                    self.stream.advance();
                    let s = name.as_str();
                    // Reject tokens carrying control characters
                    // (defensive — the lexer rules already
                    // forbid them in identifiers, but the path
                    // form is permissive enough that an
                    // explicit guard is cheap insurance).
                    if s.contains('\0') || s.contains('\n') || s.contains('\r') {
                        return Err(ParseError::invalid_mount_syntax(
                            "file-relative mount path contains a control character",
                            self.stream.current_span(),
                        ));
                    }
                    path.push_str(s);
                    segment_buf.push_str(s);
                    last_was_separator = false;
                }
                Some(TokenKind::Slash) => {
                    self.stream.advance();
                    if last_was_separator {
                        return Err(ParseError::invalid_mount_syntax(
                            "double `/` in file-relative mount path",
                            self.stream.current_span(),
                        ));
                    }
                    // Update body depth based on the segment
                    // we just closed.  `..` decrements (and
                    // must not push us below zero — that's
                    // syntactic escape that the parser
                    // rejects); `.` is identity; anything
                    // else is a real directory step.
                    if segment_buf == ".." {
                        body_depth -= 1;
                        if body_depth < 0 {
                            return Err(ParseError::invalid_mount_syntax(
                                "file-relative mount path escapes the source directory \
                                 (too many `..` segments after the leading prefix)",
                                self.stream.current_span(),
                            ));
                        }
                    } else if !segment_buf.is_empty() && segment_buf != "." {
                        body_depth += 1;
                    }
                    segment_buf.clear();
                    path.push('/');
                    last_was_separator = true;
                }
                Some(TokenKind::Dot) => {
                    self.stream.advance();
                    path.push('.');
                    segment_buf.push('.');
                    last_was_separator = false;
                }
                Some(TokenKind::DotDot) => {
                    self.stream.advance();
                    path.push_str("..");
                    segment_buf.push_str("..");
                    last_was_separator = false;
                }
                // Terminators — `as`, `;`, `,`, `}` end the
                // path and surface back to the surrounding
                // parser.
                Some(TokenKind::As)
                | Some(TokenKind::Semicolon)
                | Some(TokenKind::Comma)
                | Some(TokenKind::RBrace)
                | None => break,
                _ => {
                    return Err(ParseError::invalid_mount_syntax(
                        "unexpected token in file-relative mount path",
                        self.stream.current_span(),
                    ));
                }
            }
        }

        // Final validation.
        if !path.ends_with(".vr") {
            return Err(ParseError::invalid_mount_syntax(
                "file-relative mount path must end with `.vr`",
                self.stream.current_span(),
            ));
        }
        // The terminal segment must not itself be `..` or `.`.
        if segment_buf == ".." || segment_buf == "." {
            return Err(ParseError::invalid_mount_syntax(
                "file-relative mount path must terminate at a `.vr` file, not a directory marker",
                self.stream.current_span(),
            ));
        }

        let span = self.stream.make_span(start_pos);
        Ok(MountTree {
            kind: MountTreeKind::File {
                path: verum_common::Text::from(path),
                span,
            },
            alias: Maybe::None,
            span,
        })
    }

    /// Parse a predicate declaration.
    fn parse_predicate(&mut self, vis: Visibility) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // "predicate" keyword (contextual)
        if let Some(Token {
            kind: TokenKind::Ident(name),
            ..
        }) = self.stream.peek()
        {
            if name.as_str() != "predicate" {
                return Err(ParseError::invalid_syntax(
                    "expected 'predicate'",
                    self.stream.current_span(),
                ));
            }
            self.stream.advance();
        } else {
            return Err(ParseError::invalid_syntax(
                "expected 'predicate'",
                self.stream.current_span(),
            ));
        }

        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        // Optional generic parameters: <T, U>
        let generics: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?
        } else {
            List::new()
        };

        let params = self.parse_function_params()?;

        self.stream.expect(TokenKind::RArrow)?;
        // Use parse_type_with_lookahead to avoid consuming the body { as a refinement
        let return_type = self.parse_type_with_lookahead()?;

        // Parse predicate body as a block expression (supports let bindings + final expression)
        let body = self.parse_block_expr()?;

        let span = self.stream.make_span(start_pos);
        Ok(Item::new(
            ItemKind::Predicate(PredicateDecl {
                visibility: vis,
                name: Ident::new(name, name_span),
                generics,
                params: params.into_iter().collect(),
                return_type,
                body: Box::new(body),
                span,
            }),
            span,
        ))
    }

    /// Parse a meta (macro) declaration.
    ///
    /// Syntax:
    /// ```text
    /// meta name(param1: fragment, param2: fragment) {
    ///     pattern => expansion |
    ///     pattern => expansion
    /// }
    /// ```
    ///
    /// Where fragment can be: expr, stmt, type, pattern, ident, path, tt, item, block
    fn parse_meta(&mut self, vis: Visibility) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // 'meta' keyword
        self.stream.expect(TokenKind::Meta)?;

        // Meta name
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        // Parameters: (param1: fragment, param2: fragment)
        self.stream.expect(TokenKind::LParen)?;
        let params = if self.stream.check(&TokenKind::RParen) {
            Vec::new()
        } else {
            self.comma_separated(|p| p.parse_meta_param())?
        };
        self.stream.expect(TokenKind::RParen)?;

        // Rules: { pattern => expansion | pattern => expansion }
        self.stream.expect(TokenKind::LBrace)?;
        let rules = self.parse_meta_rules()?;
        self.stream.expect(TokenKind::RBrace)?;

        let span = self.stream.make_span(start_pos);
        Ok(Item::new(
            ItemKind::Meta(MetaDecl {
                visibility: vis,
                name: Ident::new(name, name_span),
                params: params.into_iter().collect(),
                rules: rules.into_iter().collect(),
                span,
            }),
            span,
        ))
    }

    /// Parse a meta parameter: name: fragment
    fn parse_meta_param(&mut self) -> ParseResult<MetaParam> {
        let start_pos = self.stream.position();
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        // Optional fragment specifier: :fragment
        let fragment = if self.stream.consume(&TokenKind::Colon).is_some() {
            Maybe::Some(self.parse_meta_fragment()?)
        } else {
            Maybe::None
        };

        let span = self.stream.make_span(start_pos);
        Ok(MetaParam {
            name: Ident::new(name, name_span),
            fragment,
            span,
        })
    }

    /// Parse a meta fragment specifier.
    fn parse_meta_fragment(&mut self) -> ParseResult<MetaFragment> {
        use verum_ast::decl::MetaFragment;

        if let Some(Token {
            kind: TokenKind::Ident(name),
            ..
        }) = self.stream.peek()
        {
            let fragment = match name.as_str() {
                "expr" => MetaFragment::Expr,
                "stmt" => MetaFragment::Stmt,
                "type" => MetaFragment::Type,
                "pattern" => MetaFragment::Pattern,
                "ident" => MetaFragment::Ident,
                "path" => MetaFragment::Path,
                "tt" => MetaFragment::TokenTree,
                "item" => MetaFragment::Item,
                "block" => MetaFragment::Block,
                _ => {
                    return Err(ParseError::invalid_syntax(
                        "expected meta fragment (expr, stmt, type, pattern, ident, path, tt, item, block)",
                        self.stream.current_span(),
                    ));
                }
            };
            self.stream.advance();
            Ok(fragment)
        } else {
            Err(ParseError::invalid_syntax(
                "expected meta fragment specifier",
                self.stream.current_span(),
            ))
        }
    }

    /// Parse meta rules: pattern => expansion | pattern => expansion
    fn parse_meta_rules(&mut self) -> ParseResult<Vec<MetaRule>> {
        use verum_ast::decl::MetaRule;

        let mut rules = Vec::new();

        // Parse first rule
        if !self.stream.check(&TokenKind::RBrace) {
            rules.push(self.parse_meta_rule()?);

            // Parse additional rules separated by |
            while self.stream.consume(&TokenKind::Pipe).is_some() {
                // Allow trailing pipe
                if self.stream.check(&TokenKind::RBrace) {
                    break;
                }
                rules.push(self.parse_meta_rule()?);
            }
        }

        Ok(rules)
    }

    /// Parse a single meta rule: pattern => expansion
    fn parse_meta_rule(&mut self) -> ParseResult<MetaRule> {
        use verum_ast::decl::MetaRule;

        let start_pos = self.stream.position();

        // Parse pattern
        let pattern = self.parse_pattern()?;

        // '=>' separator
        self.stream.expect(TokenKind::FatArrow)?;

        // Parse expansion: either a block `{ ... }` or an expression.
        // Block syntax prevents `|` from being consumed as binary operator.
        // Expression syntax allows `if ... { } else { }` without extra braces.
        // Enable $ident splice references inside meta rule bodies.
        let prev_in_meta = self.in_meta_body;
        self.in_meta_body = true;
        let expansion = if self.stream.check(&TokenKind::LBrace) {
            let block = self.parse_block()?;
            let span = self.stream.make_span(start_pos);
            Expr::new(ExprKind::Block(block), span)
        } else {
            self.parse_expr()?
        };
        self.in_meta_body = prev_in_meta;

        let span = self.stream.make_span(start_pos);
        Ok(MetaRule {
            pattern,
            expansion,
            span,
        })
    }

    /// Parse a context declaration.
    ///
    /// Supports both sync and async contexts:
    /// - `context Database { ... }` (sync)
    /// - `async context Database { ... }` (async)
    /// - `context async Database { ... }` (async, alternative syntax)
    ///
    /// Note: The caller may have already consumed 'context' and 'async' keywords
    /// depending on the parsing path taken.
    fn parse_context(&mut self, vis: Visibility, is_async: bool) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // Only consume 'context' if not already consumed by caller
        if self.stream.check(&TokenKind::Context) {
            self.stream.expect(TokenKind::Context)?;
        }

        // E058: Missing context name - check if '{' comes immediately
        if self.stream.check(&TokenKind::LBrace) {
            return Err(ParseError::missing_context_name(self.stream.current_span()));
        }

        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        // Parse optional generic parameters: context State<S> { ... }
        let generics: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?
        } else {
            List::new()
        };

        // Check for shorthand form: context Name: Type;
        // Defines a context that holds a single value of the given type
        if self.stream.check(&TokenKind::Colon) {
            self.stream.advance(); // consume ':'
            let ty = self.parse_type()?;
            self.stream.consume(&TokenKind::Semicolon);
            let span = self.stream.make_span(start_pos);
            return Ok(Item::new(
                ItemKind::Context(ContextDecl {
                    visibility: vis,
                    name: Ident::new(name, name_span),
                    generics,
                    methods: List::new(),
                    sub_contexts: List::new(),
                    associated_types: List::new(),
                    associated_consts: List::new(),
                    is_async,
                    span,
                }),
                span,
            ));
        }

        // E059: Missing context body - check if '{' is missing
        if !self.stream.check(&TokenKind::LBrace) {
            return Err(ParseError::missing_context_body(self.stream.current_span()));
        }
        self.stream.advance(); // consume '{'

        // Parse methods and sub-contexts
        // Sub-contexts enable fine-grained capabilities (e.g., FS.ReadOnly, FS.WriteOnly)
        // Sub-contexts are compile-time only with zero runtime overhead.
        let mut methods = Vec::new();
        let mut sub_contexts = Vec::new();

        while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }

            // First parse any attributes (e.g., @deprecated, @inline)
            let method_attrs = self.parse_attributes()?;

            // Check for sub-context declaration: context SubName { ... }
            // or async context SubName { ... }
            if self.stream.check(&TokenKind::Context) {
                // Sub-context found - recursively parse it
                let sub_ctx_item = self.parse_context(Visibility::Public, false)?;
                if let ItemKind::Context(sub_ctx) = sub_ctx_item.kind {
                    sub_contexts.push(sub_ctx);
                }
            } else if self.stream.check(&TokenKind::Async) {
                // Could be async context or async fn - peek ahead
                let next = self.stream.peek_nth(1);
                if matches!(next, Some(tok) if tok.kind == TokenKind::Context) {
                    // async context SubName { ... }
                    self.stream.advance(); // consume 'async'
                    let sub_ctx_item = self.parse_context(Visibility::Public, true)?;
                    if let ItemKind::Context(sub_ctx) = sub_ctx_item.kind {
                        sub_contexts.push(sub_ctx);
                    }
                } else {
                    // async fn method with attributes
                    methods.push(self.parse_context_method_with_attrs(method_attrs)?);
                }
            } else {
                // Regular method with attributes
                methods.push(self.parse_context_method_with_attrs(method_attrs)?);
            }
        }

        self.stream.expect(TokenKind::RBrace)?;

        let span = self.stream.make_span(start_pos);
        Ok(Item::new(
            ItemKind::Context(ContextDecl {
                visibility: vis,
                is_async,
                name: Ident::new(name, name_span),
                generics,
                methods: methods.into_iter().collect(),
                sub_contexts: sub_contexts.into_iter().collect(),
                associated_types: List::new(),
                associated_consts: List::new(),
                span,
            }),
            span,
        ))
    }

    /// Parse a context method signature.
    ///
    /// Supports both sync and async methods:
    /// - `fn query(sql: Text) -> Result<Rows>` (sync)
    /// - `async fn query(sql: Text) -> Result<Rows>` (async)
    ///
    /// Also supports attributes on methods:
    /// - `@deprecated("Use X instead") fn old_method() -> T;`
    fn parse_context_method_with_attrs(&mut self, attrs: Vec<Attribute>) -> ParseResult<FunctionDecl> {
        let start_pos = self.stream.position();

        // Check for optional async keyword
        let is_async = self.stream.consume(&TokenKind::Async).is_some();

        self.stream.expect(TokenKind::Fn)?;

        // E060: Missing function name - `fn;` or `fn(`
        if self.stream.check(&TokenKind::Semicolon)
            || self.stream.check(&TokenKind::LParen)
            || self.stream.check(&TokenKind::RBrace)
        {
            return Err(ParseError::invalid_context_method(
                "missing function name after 'fn'",
                self.stream.current_span(),
            ));
        }

        // Allow keywords as function names (e.g., `show` which is a proof keyword)
        let name = self.consume_ident_or_any_keyword()?;
        let name_span = self.stream.current_span();

        // E060: Missing parentheses - `fn connect missing_paren`
        if !self.stream.check(&TokenKind::LParen) && !self.stream.check(&TokenKind::Lt) {
            return Err(ParseError::invalid_context_method(
                "expected '(' or '<' after function name",
                self.stream.current_span(),
            ));
        }

        let generics: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?
        } else {
            List::new()
        };

        // E060: Check for invalid parameter (literal)
        if self.stream.check(&TokenKind::LParen) {
            if let Some(TokenKind::Integer(_)) | Some(TokenKind::Float(_)) | Some(TokenKind::Text(_)) =
                self.stream.peek_nth(1).map(|t| &t.kind)
            {
                return Err(ParseError::invalid_context_method(
                    "invalid parameter: expected pattern, found literal",
                    self.stream.peek_nth(1).map(|t| t.span).unwrap_or(self.stream.current_span()),
                ));
            }
        }

        let params = self.parse_function_params()?;

        // Context methods don't have bodies, so no lookahead needed
        // But we use parse_type_with_lookahead for consistency
        let return_type = if self.stream.consume(&TokenKind::RArrow).is_some() {
            Maybe::Some(self.parse_type_with_lookahead()?)
        } else {
            Maybe::None
        };

        // Parse optional using clause
        // Support both: using [Database, Logger] and using Database
        let contexts = if self.stream.consume(&TokenKind::Using).is_some() {
            self.parse_using_contexts()?
        } else if self.stream.check(&TokenKind::LBracket) {
            self.parse_context_requirements()?
        } else {
            Vec::new()
        };

        // E060: Context methods are signatures, so they end with semicolon
        // Spec: grammar/verum.ebnf - context_function ends with ';'
        if !self.stream.check(&TokenKind::Semicolon) {
            return Err(ParseError::invalid_context_method(
                "expected ';' at end of context method signature",
                self.stream.current_span(),
            ));
        }
        self.stream.advance();

        let span = self.stream.make_span(start_pos);
        Ok(FunctionDecl {
            visibility: Visibility::Public,
            is_async,
            is_meta: false,
            stage_level: 0,  // Closure signatures are runtime (stage 0)
            is_pure: false,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,  // Closures cannot be transparent
            extern_abi: Maybe::None,
            is_variadic: false,
            name: Ident::new(name, name_span),
            generics,
            params: params.into_iter().collect(),
            return_type,
            throws_clause: Maybe::None,
            std_attr: None,
            contexts: contexts.into_iter().collect(),
            generic_where_clause: None,
            meta_where_clause: None,
            requires: List::new(),
            ensures: List::new(),
            attributes: attrs.into_iter().collect(),
            body: None,
            span,
        })
    }

    /// Parse a context group declaration.
    fn parse_context_group(&mut self, vis: Visibility) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        self.stream.expect(TokenKind::Context)?;

        // "group" keyword (contextual)
        if let Some(Token {
            kind: TokenKind::Ident(name),
            ..
        }) = self.stream.peek()
        {
            if name.as_str() != "group" {
                return Err(ParseError::invalid_syntax(
                    "expected 'group'",
                    self.stream.current_span(),
                ));
            }
            self.stream.advance();
        } else {
            return Err(ParseError::invalid_syntax(
                "expected 'group'",
                self.stream.current_span(),
            ));
        }

        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        self.stream.expect(TokenKind::LBrace)?;

        // Parse comma-separated list of full context requirements
        // Supports: !IO, State<_>, Database.transactional(), etc.
        let contexts = if self.stream.check(&TokenKind::RBrace) {
            Vec::new()
        } else {
            self.comma_separated(|p| p.parse_extended_context_item())?
        };

        self.stream.expect(TokenKind::RBrace)?;

        let span = self.stream.make_span(start_pos);
        Ok(Item::new(
            ItemKind::ContextGroup(ContextGroupDecl {
                visibility: vis,
                name: Ident::new(name, name_span),
                contexts: contexts.into_iter().collect(),
                span,
            }),
            span,
        ))
    }

    /// Parse a context group alias declaration.
    ///
    /// # Syntax
    /// ```text
    /// using Name = [Context1, Context2, ...]
    /// ```
    ///
    /// This is an alternative syntax to:
    /// ```text
    /// context group Name { Context1, Context2, ... }
    /// ```
    fn parse_context_group_alias(&mut self, vis: Visibility) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // using keyword
        self.stream.expect(TokenKind::Using)?;

        // Group name
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        // = token
        self.stream.expect(TokenKind::Eq)?;

        // [ token
        self.stream.expect(TokenKind::LBracket)?;

        // Parse comma-separated list of full context requirements
        // Supports: !IO, State<_>, Database.transactional(), etc.
        let contexts = if self.stream.check(&TokenKind::RBracket) {
            Vec::new()
        } else {
            self.comma_separated(|p| p.parse_extended_context_item())?
        };

        // ] token
        self.stream.expect(TokenKind::RBracket)?;

        // ; token (required by grammar: context_group_def = 'using' ... ';')
        self.stream.expect(TokenKind::Semicolon)?;

        let span = self.stream.make_span(start_pos);
        Ok(Item::new(
            ItemKind::ContextGroup(ContextGroupDecl {
                visibility: vis,
                name: Ident::new(name, name_span),
                contexts: contexts.into_iter().collect(),
                span,
            }),
            span,
        ))
    }

    /// Parse a module-level context requirement: `using [Context1, Context2]`
    /// This declares contexts that all functions in the module require.
    /// Parsed as a ContextGroup with the implicit name "__module_contexts__".
    fn parse_module_context_requirement(&mut self, vis: Visibility) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // using keyword
        self.stream.expect(TokenKind::Using)?;

        // [ token
        self.stream.expect(TokenKind::LBracket)?;

        // Parse comma-separated list of context requirements
        let contexts = if self.stream.check(&TokenKind::RBracket) {
            Vec::new()
        } else {
            self.comma_separated(|p| p.parse_extended_context_item())?
        };

        // ] token
        self.stream.expect(TokenKind::RBracket)?;

        // Optional semicolon (top-level using doesn't always have one)
        self.stream.consume(&TokenKind::Semicolon);

        let span = self.stream.make_span(start_pos);
        let name_span = span;
        Ok(Item::new(
            ItemKind::ContextGroup(ContextGroupDecl {
                visibility: vis,
                name: Ident::new(Text::from("__module_contexts__"), name_span),
                contexts: contexts.into_iter().collect(),
                span,
            }),
            span,
        ))
    }

    /// Parse a context layer declaration.
    ///
    /// # Grammar
    /// ```text
    /// layer_def  = visibility 'layer' identifier layer_body ;
    /// layer_body = '{' { provide_stmt } '}'
    ///            | '=' layer_expr ';' ;
    /// layer_expr = identifier { '+' identifier } ;
    /// ```
    ///
    /// # Examples
    /// ```verum
    /// layer DatabaseLayer {
    ///     provide ConnectionPool = ConnectionPool.new(Config.get_url());
    ///     provide QueryExecutor = QueryExecutor.new(ConnectionPool);
    /// }
    /// layer AppLayer = DatabaseLayer + LoggingLayer;
    /// ```
    fn parse_layer(
        &mut self,
        attrs: Vec<Attribute>,
        vis: Visibility,
    ) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // Consume 'layer' keyword
        self.stream.expect(TokenKind::Layer)?;

        // Layer name
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        let kind = if self.stream.check(&TokenKind::LBrace) {
            // Inline layer: layer Name { provide ... ; ... }
            self.stream.expect(TokenKind::LBrace)?;

            let mut provides = Vec::new();
            while !self.stream.check(&TokenKind::RBrace) {
                // Each item is a provide statement:
                // 'provide' identifier '=' expr ';'
                self.stream.expect(TokenKind::Provide)?;
                let ctx_name = self.consume_ident()?;
                let ctx_span = self.stream.current_span();
                self.stream.expect(TokenKind::Eq)?;
                let value = self.parse_expr()?;
                self.stream.expect(TokenKind::Semicolon)?;
                provides.push((Ident::new(ctx_name, ctx_span), value));
            }

            self.stream.expect(TokenKind::RBrace)?;

            LayerKind::Inline {
                provides: provides.into_iter().collect(),
            }
        } else if self.stream.consume(&TokenKind::Eq).is_some() {
            // Composite layer: layer Name = Layer1 + Layer2 + ... ;
            let first = self.consume_ident()?;
            let first_span = self.stream.current_span();
            let mut layers = vec![Ident::new(first, first_span)];

            while self.stream.consume(&TokenKind::Plus).is_some() {
                let next = self.consume_ident()?;
                let next_span = self.stream.current_span();
                layers.push(Ident::new(next, next_span));
            }

            self.stream.expect(TokenKind::Semicolon)?;

            LayerKind::Composite {
                layers: layers.into_iter().collect(),
            }
        } else {
            return Err(ParseError::invalid_syntax(
                "expected '{' or '=' after layer name",
                self.stream.current_span(),
            ));
        };

        let span = self.stream.make_span(start_pos);
        Ok(Item::new_with_attrs(
            ItemKind::Layer(LayerDecl {
                visibility: vis,
                name: Ident::new(name, name_span),
                kind,
                span,
            }),
            attrs.into(),
            span,
        ))
    }

    /// Parse an FFI boundary declaration.
    ///
    /// # Arguments
    ///
    /// * `attrs` - Attributes parsed before the ffi keyword, including cfg conditions.
    ///
    /// # Platform-Specific Boundaries
    ///
    /// FFI boundaries can have cfg attributes for platform-specific code:
    ///
    /// ```verum
    /// #[cfg(target_os = "windows")]
    /// ffi Kernel32 {
    ///     @extern("C", calling_convention = "stdcall")
    ///     fn CreateFileW(...) -> *void;
    /// }
    /// ```
    fn parse_ffi_boundary(&mut self, attrs: Vec<Attribute>) -> ParseResult<Item> {
        let start_pos = self.stream.position();
        let vis = self.parse_visibility()?;

        self.stream.expect(TokenKind::Ffi)?;
        // Allow keywords as FFI boundary names (e.g., `ffi internal { ... }`)
        let name = self.consume_ident_or_any_keyword()?;
        let name_span = self.stream.current_span();

        // Parse optional 'extends' clause
        let extends = if self.stream.check(&TokenKind::Extends) {
            self.stream.advance();
            let parent_name = self.consume_ident()?;
            let parent_span = self.stream.current_span();
            Maybe::Some(Ident::new(parent_name, parent_span))
        } else {
            Maybe::None
        };

        self.stream.expect(TokenKind::LBrace)?;

        let mut functions = Vec::new();
        while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }
            // Skip const declarations inside FFI blocks (e.g., error codes)
            // They are valid per EBNF but we don't store them in the FFI AST yet
            if self.stream.check(&TokenKind::Const) {
                // Skip: const NAME: TYPE = VALUE;
                while !self.stream.check(&TokenKind::Semicolon)
                    && !self.stream.check(&TokenKind::RBrace)
                    && !self.stream.at_end()
                {
                    self.stream.advance();
                }
                self.stream.consume(&TokenKind::Semicolon);
                continue;
            }
            // Skip type declarations inside FFI blocks (handles nested braces)
            if self.stream.check(&TokenKind::Type) {
                let mut brace_depth = 0u32;
                loop {
                    if self.stream.at_end() {
                        break;
                    }
                    if brace_depth == 0 && self.stream.check(&TokenKind::Semicolon) {
                        break;
                    }
                    if brace_depth == 0 && self.stream.check(&TokenKind::RBrace) {
                        break;
                    }
                    if self.stream.check(&TokenKind::LBrace) {
                        brace_depth += 1;
                    } else if self.stream.check(&TokenKind::RBrace) {
                        brace_depth = brace_depth.saturating_sub(1);
                    }
                    self.stream.advance();
                }
                self.stream.consume(&TokenKind::Semicolon);
                continue;
            }
            functions.push(self.parse_ffi_function()?);
        }

        self.stream.expect(TokenKind::RBrace)?;

        let span = self.stream.make_span(start_pos);
        Ok(Item::new(
            ItemKind::FFIBoundary(FFIBoundary {
                name: Ident::new(name, name_span),
                extends,
                functions: functions.into(),
                visibility: vis,
                attributes: attrs.into(),
                span,
            }),
            span,
        ))
    }

    /// Parse an FFI function within a boundary.
    fn parse_ffi_function(&mut self) -> ParseResult<FFIFunction> {
        let start_pos = self.stream.position();

        // Parse attributes
        let attrs = self.parse_attributes()?;

        self.stream.expect(TokenKind::Fn)?;
        // Allow keywords as function names (e.g., `show` which is a proof keyword)
        let name = self.consume_ident_or_any_keyword()?;
        let name_span = self.stream.current_span();

        // Parameters (with variadic support)
        self.stream.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        let mut is_variadic = false;

        if !self.stream.check(&TokenKind::RParen) {
            loop {
                // Check for variadic marker `...`
                if self.stream.check(&TokenKind::DotDotDot) {
                    self.stream.advance();
                    is_variadic = true;
                    break;
                }

                // FFI parameters can use keywords as names (e.g., protocol, type, stream, etc.)
                let param_name = self.consume_ident_or_any_keyword()?;
                let param_name_span = self.stream.current_span();
                self.stream.expect(TokenKind::Colon)?;
                let ty = self.parse_type()?;
                params.push((Ident::new(param_name, param_name_span), ty));

                // Check for comma (more params) or end
                if self.stream.consume(&TokenKind::Comma).is_none() {
                    break;
                }

                // After comma, check for variadic or trailing comma before `)`
                if self.stream.check(&TokenKind::DotDotDot) {
                    self.stream.advance();
                    is_variadic = true;
                    break;
                }

                // Allow trailing comma
                if self.stream.check(&TokenKind::RParen) {
                    break;
                }
            }
        }
        self.stream.expect(TokenKind::RParen)?;

        // Return type
        let return_type = if self.stream.consume(&TokenKind::RArrow).is_some() {
            self.parse_type_no_refinement()?
        } else {
            Type::unit(self.stream.current_span())
        };

        // Semicolon after FFI function signature (optional if at end of block)
        if !self.stream.check(&TokenKind::RBrace) {
            self.stream.consume(&TokenKind::Semicolon);
        }

        // Parse contract clauses
        let mut requires = Vec::new();
        let mut ensures = Vec::new();
        let mut memory_effects = MemoryEffects::Pure;
        let mut thread_safe = false;
        let mut error_protocol = ErrorProtocol::None;
        let mut ownership = Ownership::Borrow;

        // Look for contract keywords (as actual keywords or identifiers)
        loop {
            let keyword = self.stream.peek_kind();

            // Check for requires keyword
            if matches!(keyword, Some(&TokenKind::Requires)) {
                self.stream.advance();
                loop {
                    let pos_before = self.stream.position();
                    let expr = self.parse_expr()?;
                    requires.push(expr);

                    // Safety: Ensure we made forward progress
                    if self.stream.position() == pos_before {
                        return Err(ParseError::invalid_syntax(
                            "parser made no progress in requires clause",
                            self.stream.current_span(),
                        ));
                    }

                    if self.stream.consume(&TokenKind::Semicolon).is_none() {
                        break;
                    }
                    // Check if next is another contract keyword, or end of function/boundary
                    if self.is_ffi_contract_keyword()
                        || self.stream.check(&TokenKind::RBrace)
                        || self.stream.check(&TokenKind::Fn)
                        || self.stream.check(&TokenKind::At)
                    {
                        break;
                    }
                }
            // Check for ensures keyword
            } else if matches!(keyword, Some(&TokenKind::Ensures)) {
                self.stream.advance();
                loop {
                    let pos_before = self.stream.position();
                    let expr = self.parse_expr()?;
                    ensures.push(expr);

                    // Safety: Ensure we made forward progress
                    if self.stream.position() == pos_before {
                        return Err(ParseError::invalid_syntax(
                            "parser made no progress in ensures clause",
                            self.stream.current_span(),
                        ));
                    }

                    if self.stream.consume(&TokenKind::Semicolon).is_none() {
                        break;
                    }
                    // Check if next is another contract keyword, or end of function/boundary
                    if self.is_ffi_contract_keyword()
                        || self.stream.check(&TokenKind::RBrace)
                        || self.stream.check(&TokenKind::Fn)
                        || self.stream.check(&TokenKind::At)
                    {
                        break;
                    }
                }
            // Check for memory_effects (identifier, not keyword)
            } else if let Some(Token {
                kind: TokenKind::Ident(kw),
                ..
            }) = self.stream.peek()
            {
                match kw.as_str() {
                    "memory_effects" => {
                        self.stream.advance();
                        self.stream.expect(TokenKind::Eq)?;
                        memory_effects = self.parse_memory_effects()?;
                        self.stream.consume(&TokenKind::Semicolon);
                    }
                    "thread_safe" => {
                        self.stream.advance();
                        self.stream.expect(TokenKind::Eq)?;
                        thread_safe = if self.stream.check(&TokenKind::True) {
                            self.stream.advance();
                            true
                        } else if self.stream.check(&TokenKind::False) {
                            self.stream.advance();
                            false
                        } else {
                            return Err(ParseError::invalid_syntax(
                                "expected 'true' or 'false'",
                                self.stream.current_span(),
                            ));
                        };
                        self.stream.consume(&TokenKind::Semicolon);
                    }
                    "errors_via" => {
                        self.stream.advance();
                        self.stream.expect(TokenKind::Eq)?;
                        error_protocol = self.parse_error_protocol()?;
                        self.stream.consume(&TokenKind::Semicolon);
                    }
                    _ => break,
                }
            } else {
                break;
            }
        }

        // Check for @ownership attribute - parse directly instead of using parse_attributes()
        // because looks_like_meta_expression_not_attribute() incorrectly rejects @ownership(...);
        // Use lookahead to check if it's @ownership before consuming tokens
        let is_at_ownership = {
            let mut stream_clone = self.stream.clone();
            if stream_clone.check(&TokenKind::At) {
                stream_clone.advance();
                matches!(
                    stream_clone.peek(),
                    Some(Token { kind: TokenKind::Ident(name), .. }) if name.as_str() == "ownership"
                )
            } else {
                false
            }
        };

        if is_at_ownership {
            let attr_start = self.stream.position();
            self.stream.advance(); // consume @
            let name_text = Text::from("ownership");
            self.stream.advance(); // consume 'ownership'

            // Parse arguments
            let args = if self.stream.consume(&TokenKind::LParen).is_some() {
                let exprs = if self.stream.check(&TokenKind::RParen) {
                    Vec::new()
                } else {
                    self.comma_separated(|p| p.parse_attribute_arg())?
                };
                self.stream.expect(TokenKind::RParen)?;
                Some(exprs)
            } else {
                None
            };

            let attr_span = self.stream.make_span(attr_start);
            let attr = Attribute {
                name: name_text,
                args: args.map(|v| v.into()),
                span: attr_span,
            };
            ownership = self.extract_ownership(&attr)?;

            // Consume optional trailing semicolon
            self.stream.consume(&TokenKind::Semicolon);
        }

        let calling_convention = self.extract_calling_convention(&attrs);
        let span = self.stream.make_span(start_pos);

        Ok(FFIFunction {
            name: Ident::new(name, name_span),
            signature: FFISignature {
                params: params.into_iter().collect(),
                return_type,
                calling_convention,
                is_variadic,
                span,
            },
            requires: requires.into(),
            ensures: ensures.into(),
            memory_effects,
            thread_safe,
            error_protocol,
            ownership,
            span,
        })
    }

    /// Check if current token is an FFI contract keyword
    fn is_ffi_contract_keyword(&self) -> bool {
        match self.stream.peek_kind() {
            Some(&TokenKind::Requires) | Some(&TokenKind::Ensures) => true,
            Some(TokenKind::Ident(name)) => {
                matches!(
                    name.as_str(),
                    "memory_effects" | "thread_safe" | "errors_via"
                )
            }
            _ => false,
        }
    }

    /// Parse memory effects.
    fn parse_memory_effects(&mut self) -> ParseResult<MemoryEffects> {
        // Parse a single memory effect
        let first_effect = self.parse_single_memory_effect()?;

        // Check for + operator to combine effects
        if self.stream.check(&TokenKind::Plus) {
            let mut effects = vec![first_effect];
            while self.stream.consume(&TokenKind::Plus).is_some() {
                effects.push(self.parse_single_memory_effect()?);
            }
            Ok(MemoryEffects::Combined(effects.into_iter().collect()))
        } else {
            Ok(first_effect)
        }
    }

    /// Parse a single memory effect (without + operator).
    fn parse_single_memory_effect(&mut self) -> ParseResult<MemoryEffects> {
        if let Some(Token {
            kind: TokenKind::Ident(name),
            ..
        }) = self.stream.peek()
        {
            match name.as_str() {
                "Pure" => {
                    self.stream.advance();
                    Ok(MemoryEffects::Pure)
                }
                "Allocates" => {
                    self.stream.advance();
                    Ok(MemoryEffects::Allocates)
                }
                "Reads" => {
                    self.stream.advance();
                    // Check for optional parameter list: Reads(param1, param2)
                    if self.stream.consume(&TokenKind::LParen).is_some() {
                        let mut params = Vec::new();
                        if !self.stream.check(&TokenKind::RParen) {
                            loop {
                                // Allow keywords as parameter names (e.g., stream)
                                let param = self.consume_ident_or_any_keyword()?;
                                params.push(param);
                                if self.stream.consume(&TokenKind::Comma).is_none() {
                                    break;
                                }
                            }
                        }
                        self.stream.expect(TokenKind::RParen)?;
                        Ok(MemoryEffects::Reads(if params.is_empty() {
                            None
                        } else {
                            Some(params.into_iter().collect())
                        }))
                    } else {
                        Ok(MemoryEffects::Reads(None))
                    }
                }
                "Writes" => {
                    self.stream.advance();
                    // Check for optional parameter list: Writes(param1, param2)
                    if self.stream.consume(&TokenKind::LParen).is_some() {
                        let mut params = Vec::new();
                        if !self.stream.check(&TokenKind::RParen) {
                            loop {
                                // Allow keywords as parameter names
                                let param = self.consume_ident_or_any_keyword()?;
                                params.push(param);
                                if self.stream.consume(&TokenKind::Comma).is_none() {
                                    break;
                                }
                            }
                        }
                        self.stream.expect(TokenKind::RParen)?;
                        Ok(MemoryEffects::Writes(if params.is_empty() {
                            None
                        } else {
                            Some(params.into_iter().collect())
                        }))
                    } else {
                        Ok(MemoryEffects::Writes(None))
                    }
                }
                "Deallocates" => {
                    self.stream.advance();
                    // Check for optional parameter: Deallocates(ptr)
                    if self.stream.consume(&TokenKind::LParen).is_some() {
                        let param = if !self.stream.check(&TokenKind::RParen) {
                            // Allow keywords as parameter names
                            Some(self.consume_ident_or_any_keyword()?)
                        } else {
                            None
                        };
                        self.stream.expect(TokenKind::RParen)?;
                        Ok(MemoryEffects::Deallocates(param))
                    } else {
                        Ok(MemoryEffects::Deallocates(None))
                    }
                }
                "Combined" => {
                    self.stream.advance();
                    // Parse list of effects: Combined(effect1, effect2, ...)
                    self.stream.expect(TokenKind::LParen)?;
                    let mut effects = Vec::new();
                    if !self.stream.check(&TokenKind::RParen) {
                        loop {
                            effects.push(self.parse_single_memory_effect()?);
                            if self.stream.consume(&TokenKind::Comma).is_none() {
                                break;
                            }
                        }
                    }
                    self.stream.expect(TokenKind::RParen)?;
                    Ok(MemoryEffects::Combined(effects.into_iter().collect()))
                }
                _ => Err(ParseError::invalid_syntax(
                    "expected memory effect (Pure, Allocates, Reads, Writes, Deallocates, Combined)",
                    self.stream.current_span(),
                )),
            }
        } else {
            Err(ParseError::invalid_syntax(
                "expected memory effect",
                self.stream.current_span(),
            ))
        }
    }

    /// Parse error protocol.
    fn parse_error_protocol(&mut self) -> ParseResult<ErrorProtocol> {
        // Check for None keyword first
        if self.stream.check(&TokenKind::None) {
            self.stream.advance();
            return Ok(ErrorProtocol::None);
        }

        // Check for identifier-based protocols
        if let Some(Token {
            kind: TokenKind::Ident(name),
            ..
        }) = self.stream.peek()
        {
            match name.as_str() {
                "Errno" => {
                    self.stream.advance();
                    Ok(ErrorProtocol::Errno)
                }
                "Exception" => {
                    self.stream.advance();
                    Ok(ErrorProtocol::Exception)
                }
                "ReturnCode" => {
                    self.stream.advance();
                    self.stream.expect(TokenKind::LParen)?;
                    // Handle `== 0` shorthand (comparison without LHS)
                    if self.stream.check(&TokenKind::EqEq) {
                        self.stream.advance(); // consume ==
                    }
                    let expr = self.parse_expr()?;
                    self.stream.expect(TokenKind::RParen)?;
                    Ok(ErrorProtocol::ReturnCode(expr))
                }
                "ReturnValue" => {
                    self.stream.advance();
                    self.stream.expect(TokenKind::LParen)?;
                    let expr = self.parse_expr()?;
                    self.stream.expect(TokenKind::RParen)?;

                    // Check if followed by "with Errno" - note: 'with' is a keyword
                    if self.stream.check(&TokenKind::With) {
                        self.stream.advance();
                        if let Some(Token {
                            kind: TokenKind::Ident(errno_kw),
                            ..
                        }) = self.stream.peek()
                            && errno_kw.as_str() == "Errno"
                        {
                            self.stream.advance();
                            return Ok(ErrorProtocol::ReturnValueWithErrno(Box::new(expr)));
                        }
                    }

                    Ok(ErrorProtocol::ReturnValue(expr))
                }
                _ => Err(ParseError::invalid_syntax(
                    "expected error protocol (None, Errno, Exception, ReturnCode, ReturnValue)",
                    self.stream.current_span(),
                )),
            }
        } else {
            Err(ParseError::invalid_syntax(
                "expected error protocol",
                self.stream.current_span(),
            ))
        }
    }

    /// Extract calling convention from @extern attribute.
    ///
    /// Grammar: @extern("C") or @extern("C", calling_convention = "stdcall")
    fn extract_calling_convention(&self, attrs: &[Attribute]) -> CallingConvention {
        for attr in attrs {
            if attr.name.as_str() == "extern" {
                // Check for arguments
                if let Some(args) = &attr.args {
                    // First argument is the ABI string (e.g., "C")
                    // Second argument (if present) is calling_convention = "..."
                    for arg in args.iter() {
                        // Look for calling_convention = "stdcall" style assignments
                        if let ExprKind::Binary { op, left, right } = &arg.kind
                            && matches!(*op, verum_ast::BinOp::Assign)
                            && let ExprKind::Path(path) = &left.kind
                            && let Some(seg) = path.segments.first()
                            && let verum_ast::PathSegment::Name(ident) = seg
                            && ident.name.as_str() == "calling_convention"
                        {
                            // Extract string literal value
                            if let ExprKind::Literal(lit) = &right.kind
                                && let verum_ast::LiteralKind::Text(s) = &lit.kind
                            {
                                return match s.as_str() {
                                    "stdcall" => CallingConvention::StdCall,
                                    "fastcall" => CallingConvention::FastCall,
                                    "sysv64" => CallingConvention::SysV64,
                                    "system" => CallingConvention::System,
                                    "interrupt" => CallingConvention::Interrupt,
                                    "naked" => CallingConvention::Naked,
                                    _ => CallingConvention::C,
                                };
                            }
                        }
                    }
                }
                // Default to C calling convention
                return CallingConvention::C;
            }
        }
        // If no @extern attribute found, default to C
        CallingConvention::C
    }

    /// Extract ownership from @ownership attribute.
    ///
    /// Grammar: @ownership(borrow) | @ownership(transfer_to = "ptr") | @ownership(transfer_from = "ptr") | @ownership(shared)
    fn extract_ownership(&self, attr: &Attribute) -> ParseResult<Ownership> {
        if let Some(args) = &attr.args
            && let Some(first_arg) = args.first()
        {
            // Check for simple identifier: @ownership(borrow)
            if let ExprKind::Path(path) = &first_arg.kind
                && let Some(seg) = path.segments.first()
                && let verum_ast::PathSegment::Name(ident) = seg
            {
                match ident.name.as_str() {
                    "borrow" => return Ok(Ownership::Borrow),
                    "shared" => return Ok(Ownership::Shared),
                    _ => {}
                }
            }

            // Check for assignment: @ownership(transfer_to = "ptr")
            if let ExprKind::Binary { op, left, right } = &first_arg.kind
                && matches!(*op, verum_ast::BinOp::Assign)
                && let ExprKind::Path(path) = &left.kind
                && let Some(seg) = path.segments.first()
                && let verum_ast::PathSegment::Name(ident) = seg
            {
                // Extract string literal value
                if let ExprKind::Literal(lit) = &right.kind
                    && let verum_ast::LiteralKind::Text(s) = &lit.kind
                {
                    match ident.name.as_str() {
                        "transfer_to" => return Ok(Ownership::TransferTo(Text::from(s.as_str()))),
                        "transfer_from" => {
                            return Ok(Ownership::TransferFrom(Text::from(s.as_str())));
                        }
                        _ => {}
                    }
                }
            }
        }
        // Default to borrow if parsing fails
        Ok(Ownership::Borrow)
    }

    /// Parse @specialize attribute from attribute list.
    fn parse_specialize_from_attrs(
        &self,
        attrs: &[Attribute],
    ) -> Maybe<verum_ast::attr::SpecializeAttr> {
        use verum_ast::{ExprKind, attr::SpecializeAttr};

        let specialize_attr = attrs.iter().find(|attr| attr.name.as_str() == "specialize");

        match specialize_attr {
            None => None,
            Some(attr) => {
                let mut negative = false;
                let mut rank = None;
                let mut when_clause = None;

                if let Some(args) = &attr.args {
                    for arg in args.iter() {
                        match &arg.kind {
                            ExprKind::Path(path) => {
                                if let Some(seg) = path.segments.first()
                                    && let verum_ast::PathSegment::Name(ident) = seg
                                    && ident.name.as_str() == "negative"
                                {
                                    negative = true;
                                }
                            }
                            ExprKind::Binary { op, left, right }
                                if matches!(*op, verum_ast::BinOp::Assign) =>
                            {
                                if let ExprKind::Path(path) = &left.kind
                                    && let Some(seg) = path.segments.first()
                                    && let verum_ast::PathSegment::Name(ident) = seg
                                    && ident.name.as_str() == "rank"
                                {
                                    // Handle both positive and negative integers
                                    match &right.kind {
                                        // Positive integer literal: rank = 5
                                        ExprKind::Literal(lit) => {
                                            if let verum_ast::LiteralKind::Int(int_lit) = &lit.kind
                                            {
                                                rank = Some(int_lit.value as i32);
                                            }
                                        }
                                        // Negative integer: rank = -5
                                        ExprKind::Unary { op, expr } => {
                                            if matches!(op, verum_ast::UnOp::Neg)
                                                && let ExprKind::Literal(lit) = &expr.kind
                                                && let verum_ast::LiteralKind::Int(int_lit) =
                                                    &lit.kind
                                            {
                                                rank = Some(-(int_lit.value as i32));
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            // Extract when(...) clause
                            ExprKind::Call {
                                func,
                                args: call_args,
                                ..
                            } => {
                                // Check if this is a when() call
                                if let ExprKind::Path(path) = &func.kind
                                    && let Some(seg) = path.segments.first()
                                    && let verum_ast::PathSegment::Name(ident) = seg
                                    && ident.name.as_str() == "when"
                                {
                                    // Extract the index from the marker argument
                                    if let Some(first_arg) = call_args.first()
                                        && let ExprKind::Path(marker_path) = &first_arg.kind
                                        && let Some(marker_seg) = marker_path.segments.first()
                                        && let verum_ast::PathSegment::Name(marker_ident) =
                                            marker_seg
                                    {
                                        let marker_name = marker_ident.name.as_str();
                                        if marker_name.starts_with("_when_clause_") {
                                            // Parse the index from "_when_clause_N"
                                            if let Ok(index) = marker_name
                                                .trim_start_matches("_when_clause_")
                                                .parse::<usize>()
                                            {
                                                // Retrieve the when clause from temporary storage
                                                if index < self.when_clauses.len() {
                                                    when_clause =
                                                        Some(self.when_clauses[index].clone());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                Some(SpecializeAttr::new(negative, rank, when_clause, attr.span))
            }
        }
    }

    /// Convert an expression to a path if possible.
    /// Handles Path expressions and Field access chains (e.g., std.database.Database).
    pub(crate) fn expr_to_path(expr: &Expr) -> Maybe<Path> {
        match &expr.kind {
            // Direct path: Database
            verum_ast::ExprKind::Path(path) => Maybe::Some(path.clone()),

            // Field access chain: std.database.Database
            verum_ast::ExprKind::Field { expr: base, field } => {
                // Recursively convert the base to a path
                if let Maybe::Some(mut base_path) = Self::expr_to_path(base) {
                    // Append the field as a new segment
                    base_path
                        .segments
                        .push(verum_ast::ty::PathSegment::Name(field.clone()));
                    Maybe::Some(base_path)
                } else {
                    Maybe::None
                }
            }

            _ => Maybe::None,
        }
    }

    /// Parse attributes: @attr or @attr(args)
    ///
    /// Note: This parses ALL @name constructs as attributes. The distinction between
    /// attributes and meta-function expressions is handled at the statement level
    /// in `looks_like_macro_call_not_attribute()` in stmt.rs.
    pub(crate) fn parse_attributes(&mut self) -> ParseResult<Vec<Attribute>> {
        let mut attrs = Vec::new();

        while self.stream.check(&TokenKind::At) {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }

            // Before consuming this @name(...), check if it's an expression statement
            // rather than an attribute. If followed by `;` or `}`, it's an expression.
            // This prevents @asm("lfence"); from being parsed as an attribute.
            if self.looks_like_meta_expression_not_attribute() {
                break;
            }

            let start_pos = self.stream.position();
            self.stream.advance();

            // E013: Missing attribute name - @(Debug) without name after @
            // Allow keywords like 'using' to be used as attribute names
            let name = match self.consume_ident_or_any_keyword() {
                Ok(name) => name,
                Err(_) => {
                    let span = self.stream.make_span(start_pos);
                    return Err(ParseError::missing_attribute_name(span));
                }
            };

            let args = if self.stream.consume(&TokenKind::LParen).is_some() {
                // E012: Invalid attribute args - detect leading comma like @derive(,)
                if self.stream.check(&TokenKind::Comma) {
                    let span = self.stream.make_span(start_pos);
                    return Err(ParseError::invalid_attribute_args(
                        "unexpected leading comma",
                        span,
                    ));
                }
                // E014: Invalid nested attribute - @attr(@nested)
                if self.stream.check(&TokenKind::At) {
                    let span = self.stream.make_span(start_pos);
                    return Err(ParseError::invalid_nested_attribute(span));
                }
                // Check for empty attribute args for specific attributes
                let is_empty = self.stream.check(&TokenKind::RParen);
                let attr_name = name.as_str();
                // E015: Empty cfg attribute
                if is_empty && attr_name == "cfg" {
                    let span = self.stream.make_span(start_pos);
                    return Err(ParseError::invalid_empty_cfg(span));
                }
                // E016: Empty requires clause
                if is_empty && attr_name == "requires" {
                    let span = self.stream.make_span(start_pos);
                    return Err(ParseError::invalid_empty_requires(span));
                }
                // E017: Empty ensures clause
                if is_empty && attr_name == "ensures" {
                    let span = self.stream.make_span(start_pos);
                    return Err(ParseError::invalid_empty_ensures(span));
                }
                let exprs = if is_empty {
                    Vec::new()
                } else {
                    self.comma_separated(|p| p.parse_attribute_arg())?
                };
                // E011: Unclosed attribute parenthesis
                if self.stream.consume(&TokenKind::RParen).is_none() {
                    let span = self.stream.make_span(start_pos);
                    return Err(ParseError::unclosed_attribute(span));
                }
                Some(exprs)
            } else {
                None
            };

            let span = self.stream.make_span(start_pos);
            attrs.push(Attribute {
                name,
                args: args.map(|v| v.into()),
                span,
            });

            // Tolerate stray ']' after attribute (from incomplete #[...] -> @... conversion)
            self.stream.consume(&TokenKind::RBracket);
        }

        Ok(attrs)
    }

    /// Parse a single attribute argument.
    /// This handles special cases like `when(T: Clone)` in @specialize attributes.
    fn parse_attribute_arg(&mut self) -> ParseResult<Expr> {
        // Check if this is a `when(...)` call - special syntax for @specialize
        if let Some(Token {
            kind: TokenKind::Ident(name),
            ..
        }) = self.stream.peek()
            && name.as_str() == "when"
        {
            let start_pos = self.stream.position();
            self.stream.advance(); // consume 'when'

            if self.stream.consume(&TokenKind::LParen).is_some() {
                // Parse the content as where predicates
                let where_clause = self.parse_when_clause_contents()?;
                self.stream.expect(TokenKind::RParen)?;

                let span = self.stream.make_span(start_pos);

                // Store the where clause in temporary storage
                let index = self.when_clauses.len();
                self.when_clauses.push(where_clause);

                // Return a Call expression with a marker that includes the index
                // Format: when(_when_clause_N) where N is the index
                return Ok(Expr::new(
                    verum_ast::ExprKind::Call {
                        func: Heap::new(Expr::new(
                            verum_ast::ExprKind::Path(Path::from_ident(Ident::new(
                                Text::from("when"),
                                span,
                            ))),
                            span,
                        )),
                        type_args: List::new(),
                        args: vec![Expr::new(
                            verum_ast::ExprKind::Path(Path::from_ident(Ident::new(
                                Text::from(format!("_when_clause_{}", index)),
                                span,
                            ))),
                            span,
                        )]
                        .into_iter()
                        .collect(),
                    },
                    span,
                ));
            }
        }

        // Special case: keywords used as identifiers in attribute arguments
        // This allows @verify(proof), @verify(static), @target(module), etc.
        if let Some(Token { kind, span }) = self.stream.peek() {
            match kind {
                TokenKind::Static
                | TokenKind::Module
                | TokenKind::Type
                | TokenKind::Fn
                | TokenKind::Const
                | TokenKind::Unsafe
                | TokenKind::Proof
                | TokenKind::Pure
                | TokenKind::Async
                | TokenKind::Where => {
                    let span = *span;
                    let keyword_name = match kind {
                        TokenKind::Static => "static",
                        TokenKind::Module => "module",
                        TokenKind::Type => "type",
                        TokenKind::Fn => "fn",
                        TokenKind::Const => "const",
                        TokenKind::Unsafe => "unsafe",
                        TokenKind::Proof => "proof",
                        TokenKind::Pure => "pure",
                        TokenKind::Async => "async",
                        TokenKind::Where => "where",
                        _ => unreachable!(),
                    };
                    self.stream.advance();
                    return Ok(Expr::new(
                        verum_ast::ExprKind::Path(Path::from_ident(Ident::new(
                            Text::from(keyword_name),
                            span,
                        ))),
                        span,
                    ));
                }
                _ => {}
            }
        }

        // Handle key: value pairs in attribute arguments (e.g., @deprecated(reason: "..."))
        // Check if this looks like ident followed by ':'
        if self.is_ident() {
            let checkpoint = self.stream.position();
            let name = self.consume_ident()?;
            if self.stream.consume(&TokenKind::Colon).is_some() {
                // Parse the value expression
                let value = self.parse_expr()?;
                let span = self.stream.make_span(checkpoint);
                // Represent as a binary assign expression: name = value
                return Ok(Expr::new(
                    verum_ast::ExprKind::Binary {
                        op: verum_ast::BinOp::Assign,
                        left: Heap::new(Expr::new(
                            verum_ast::ExprKind::Path(Path::from_ident(Ident::new(name, span))),
                            span,
                        )),
                        right: Heap::new(value),
                    },
                    span,
                ));
            }
            // Not a key: value pair, reset and parse as expression
            self.stream.reset_to(checkpoint);
        }

        // Default: parse as a regular expression
        self.parse_expr()
    }

    /// Parse the contents of a `when(...)` clause in @specialize attribute.
    /// This parses type constraints like `T: Clone`, `U: Send`, etc.
    fn parse_when_clause_contents(&mut self) -> ParseResult<WhereClause> {
        let start_pos = self.stream.position();

        if self.stream.check(&TokenKind::RParen) {
            // Empty when clause
            let span = self.stream.make_span(start_pos);
            return Ok(WhereClause::empty(span));
        }

        let predicates = self.comma_separated(|p| p.parse_when_predicate())?;
        let span = self.stream.make_span(start_pos);

        Ok(WhereClause::new(predicates.into_iter().collect(), span))
    }

    /// Parse a single predicate inside `when(...)`.
    /// This can be:
    /// - Type constraint: `T: Clone`, `T: Clone + Send`
    /// - Type equality: `T == Int`
    /// - Meta constraint: `N > 0`
    fn parse_when_predicate(&mut self) -> ParseResult<verum_ast::ty::WherePredicate> {
        let start_pos = self.stream.position();

        // Try to parse as type constraint first (T: Clone)
        // Look ahead to determine if this is a type constraint or meta constraint
        // Type constraints have the pattern: IDENT :
        // Meta constraints have the pattern: IDENT <comparison_op>
        if self.is_ident() {
            // Peek ahead to see what comes after the identifier
            let checkpoint = self.stream.position();
            self.stream.advance(); // consume identifier

            if self.stream.check(&TokenKind::Colon) {
                // Definitely a type constraint: T: Clone
                self.stream.reset_to(checkpoint);
                // Use parse_type_no_sigma to prevent `T: Clone` from being parsed as a sigma type
                // In this context, `T: Clone` means T has bound Clone, not a sigma type T where Clone
                let ty = self.parse_type_no_sigma()?;
                self.stream.expect(TokenKind::Colon)?; // consume ':'

                let bounds = self.parse_type_bounds()?;
                let span = self.stream.make_span(start_pos);

                return Ok(verum_ast::ty::WherePredicate {
                    kind: verum_ast::ty::WherePredicateKind::Type { ty, bounds },
                    span,
                });
            } else {
                // Not a type constraint, reset and try as meta constraint/expression
                self.stream.reset_to(checkpoint);
            }
        }

        // Parse as meta constraint: N > 0, N == 10, N < 10, etc.
        let constraint = self.parse_expr()?;
        let span = self.stream.make_span(start_pos);

        Ok(verum_ast::ty::WherePredicate {
            kind: verum_ast::ty::WherePredicateKind::Meta { constraint },
            span,
        })
    }

    /// Parse visibility modifier.
    fn parse_visibility(&mut self) -> ParseResult<Visibility> {
        if self.stream.check(&TokenKind::Public) || self.stream.check(&TokenKind::Pub) {
            self.stream.advance();

            // Check for restriction
            if self.stream.consume(&TokenKind::LParen).is_some() {
                if self.stream.check(&TokenKind::Cog) {
                    self.stream.advance();
                    self.stream.expect(TokenKind::RParen)?;
                    return Ok(Visibility::PublicCrate);
                }

                if self.stream.check(&TokenKind::Super) {
                    self.stream.advance();
                    self.stream.expect(TokenKind::RParen)?;
                    return Ok(Visibility::PublicSuper);
                }

                // public(in path)
                if self.stream.check(&TokenKind::In) {
                    self.stream.advance();
                    let path = self.parse_path()?;
                    self.stream.expect(TokenKind::RParen)?;
                    return Ok(Visibility::PublicIn(path));
                }

                return Err(ParseError::invalid_syntax(
                    "expected 'cog', 'super', or 'in path'",
                    self.stream.current_span(),
                ));
            }

            Ok(Visibility::Public)
        } else if self.stream.check(&TokenKind::Internal) {
            // Check if 'internal' is followed by ':' - if so, it's a field name, not visibility
            // e.g., `internal: Int` is a field named 'internal', not visibility + field
            if matches!(self.stream.peek_nth(1).map(|t| &t.kind), Some(TokenKind::Colon)) {
                return Ok(Visibility::Private);
            }
            self.stream.advance();
            Ok(Visibility::Internal)
        } else if self.stream.check(&TokenKind::Protected) {
            // Check if 'protected' is followed by ':' - if so, it's a field name, not visibility
            // e.g., `protected: Int` is a field named 'protected', not visibility + field
            if matches!(self.stream.peek_nth(1).map(|t| &t.kind), Some(TokenKind::Colon)) {
                return Ok(Visibility::Private);
            }
            self.stream.advance();
            Ok(Visibility::Protected)
        } else if self.stream.check(&TokenKind::Private) {
            self.stream.advance();
            Ok(Visibility::Private)
        } else {
            Ok(Visibility::Private)
        }
    }

    // ========================================================================
    // Helper functions
    // ========================================================================

    /// Convert a Type to a Path. Used for protocol bounds that can be either
    /// simple paths (Clone, Debug) or refined types (Int{> 0}).
    /// For refined types, we extract the base type's path.
    /// For primitives, we convert them to paths.
    fn type_to_path(&self, ty: Type) -> ParseResult<Path> {
        match ty.kind {
            TypeKind::Path(path) => Ok(path),
            TypeKind::Refined { base, .. } => self.type_to_path(*base),
            // Generic type: extract base path (e.g., Iterator from Iterator<Item = T>)
            TypeKind::Generic { base, .. } => self.type_to_path(*base),
            // Convert primitive types to paths
            TypeKind::Int => Ok(Path::from_ident(Ident::new(Text::from("Int"), ty.span))),
            TypeKind::Float => Ok(Path::from_ident(Ident::new(Text::from("Float"), ty.span))),
            TypeKind::Bool => Ok(Path::from_ident(Ident::new(Text::from("Bool"), ty.span))),
            TypeKind::Char => Ok(Path::from_ident(Ident::new(Text::from("Char"), ty.span))),
            TypeKind::Text => Ok(Path::from_ident(Ident::new(Text::from("Text"), ty.span))),
            // Existential type: `some T: Bound` - extract the bound as a path
            // This occurs in associated type bounds like `type Iter: some I: Iterator;`
            TypeKind::Existential { bounds, .. } => {
                if let Some(first_bound) = bounds.first() {
                    match &first_bound.kind {
                        verum_ast::ty::TypeBoundKind::Protocol(path) => Ok(path.clone()),
                        _ => Ok(Path::from_ident(Ident::new(Text::from("_"), ty.span))),
                    }
                } else {
                    Ok(Path::from_ident(Ident::new(Text::from("_"), ty.span)))
                }
            }
            // Refined types (all three surface forms: `T{p}`, `T where p`,
            // `n: T where p`) fall through to the `Refined` arm above; the
            // dedicated Sigma arm is gone after collapse.
            _ => Err(ParseError::invalid_syntax(
                "expected path or refined type in type bound",
                ty.span,
            )),
        }
    }

    // ========================================================================
    // View Declarations
    // ========================================================================

    /// Parse a view declaration.
    /// Syntax: view Name<T> : ParamType -> ReturnType { constructors }
    ///
    /// Views provide alternative pattern interfaces for dependent types:
    /// `view Parity : Nat -> Type { Even(n: Nat) -> Parity(2*n), Odd(n: Nat) -> Parity(2*n+1) }`
    /// Enables pattern matching via view functions rather than structural decomposition.
    pub fn parse_view_decl(&mut self, attrs: Vec<Attribute>, vis: Visibility) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // view keyword
        self.stream.expect(TokenKind::View)?;

        // View name
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        // Optional generic parameters
        let generics: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?
        } else {
            List::new()
        };

        // Colon before type signature
        self.stream.expect(TokenKind::Colon)?;

        // Parameter type
        let param_type = self.parse_type()?;

        // -> Return type
        self.stream.expect(TokenKind::RArrow)?;
        let return_type = self.parse_type()?;

        // Optional where clause
        let (generic_where, meta_where) = if self.stream.check(&TokenKind::Where) {
            let where_clause = self.parse_where_clause()?;
            self.separate_view_where_clauses(&where_clause)
        } else {
            (Maybe::None, Maybe::None)
        };

        // Constructor block
        self.stream.expect(TokenKind::LBrace)?;
        let constructors = self.parse_view_constructors()?;
        self.stream.expect(TokenKind::RBrace)?;

        let span = self.stream.make_span(start_pos);
        let decl = ViewDecl {
            visibility: vis,
            name: Ident::new(name, name_span),
            generics,
            param_type,
            return_type,
            constructors: constructors.into_iter().collect(),
            generic_where_clause: generic_where,
            meta_where_clause: meta_where,
            attributes: attrs.into_iter().collect(),
            span,
        };

        Ok(Item::new(ItemKind::View(decl), span))
    }

    /// Parse view constructors.
    fn parse_view_constructors(&mut self) -> ParseResult<Vec<ViewConstructor>> {
        let mut constructors = Vec::new();

        while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
            // Safety: prevent infinite loop
            if !self.tick() || self.is_aborted() {
                break;
            }
            constructors.push(self.parse_view_constructor()?);
            // Optional comma or semicolon
            self.stream.consume(&TokenKind::Comma);
            self.stream.consume(&TokenKind::Semicolon);
        }

        Ok(constructors)
    }

    /// Parse a single view constructor.
    fn parse_view_constructor(&mut self) -> ParseResult<ViewConstructor> {
        let start_pos = self.stream.position();

        // Constructor name
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        // Optional type parameters
        let type_params: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?
        } else {
            List::new()
        };

        // Colon before signature
        self.stream.expect(TokenKind::Colon)?;

        // Parameters: (n: Nat, ...) or just Type
        let params = if self.stream.check(&TokenKind::LParen) {
            self.parse_function_params()?
        } else {
            Vec::new()
        };

        // -> Return type
        self.stream.expect(TokenKind::RArrow)?;
        let result_type = self.parse_type()?;

        let span = self.stream.make_span(start_pos);
        Ok(ViewConstructor {
            name: Ident::new(name, name_span),
            type_params,
            params: params.into_iter().collect(),
            result_type,
            span,
        })
    }

    /// Separate generic and meta predicates from a where clause for views.
    fn separate_view_where_clauses(
        &self,
        where_clause: &WhereClause,
    ) -> (Maybe<WhereClause>, Maybe<WhereClause>) {
        let mut generic_preds = Vec::new();
        let mut meta_preds = Vec::new();

        for pred in where_clause.predicates.iter() {
            match &pred.kind {
                WherePredicateKind::Meta { .. } => meta_preds.push(pred.clone()),
                _ => generic_preds.push(pred.clone()),
            }
        }

        let generic_clause = if !generic_preds.is_empty() {
            Maybe::Some(WhereClause {
                predicates: generic_preds.into_iter().collect::<List<_>>(),
                span: where_clause.span,
            })
        } else {
            Maybe::None
        };

        let meta_clause = if !meta_preds.is_empty() {
            Maybe::Some(WhereClause {
                predicates: meta_preds.into_iter().collect::<List<_>>(),
                span: where_clause.span,
            })
        } else {
            Maybe::None
        };

        (generic_clause, meta_clause)
    }

    // =========================================================================
    // ACTIVE PATTERN PARSING
    // Active patterns: user-defined pattern decomposition functions
    // Syntax: `pattern Even(n: Int) -> Bool = n % 2 == 0;`
    // Used with `&` combinator: `Even() & Positive()` in match arms
    // =========================================================================

    /// Parse an active pattern declaration.
    ///
    /// Grammar:
    /// ```ebnf
    /// pattern_def = visibility , 'pattern' , identifier , [ pattern_type_params ]
    ///             , '(' , pattern_params , ')' , '->' , type_expr , '=' , expression , ';' ;
    /// pattern_type_params = '(' , param_list , ')' ;
    /// pattern_params = [ param , { ',' , param } ] ;
    /// ```
    ///
    /// Examples:
    /// - Simple: `pattern Even(n: Int) -> Bool = n % 2 == 0;`
    /// - Parameterized: `pattern InRange(lo: Int, hi: Int)(n: Int) -> Bool = lo <= n <= hi;`
    /// - Partial: `pattern ParseInt(s: Text) -> Maybe<Int> = s.parse_int();`
    pub fn parse_pattern_decl(
        &mut self,
        attrs: Vec<Attribute>,
        vis: Visibility,
    ) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // 'pattern' keyword
        self.stream.expect(TokenKind::ActivePattern)?;

        // Pattern name
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        // Optional generic parameters: pattern Empty<T>(list: List<T>) -> Bool
        let generics = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?
        } else {
            List::new()
        };

        // Check for parameterized pattern: pattern Name(type_params)(params)
        // First set of parentheses is optional (type parameters for parameterized patterns)
        let type_params: List<FunctionParam> = if self.stream.check(&TokenKind::LParen) {
            // We need to look ahead to see if this is:
            // 1. type_params followed by params: Name(T1, T2)(value: X)
            // 2. just params: Name(value: X)
            //
            // The key difference is that type_params are followed by another '('
            // Let's parse the first parameter list and check what follows

            // Parse the first parameter list
            let first_params = self.parse_function_params()?;

            // Check if there's another '(' after - if so, this was type_params
            if self.stream.check(&TokenKind::LParen) {
                // This was type_params, now parse the actual params
                first_params.into_iter().collect()
            } else {
                // This was already the pattern params, not type_params
                // We need to put them back somehow... but that's complex.
                // Instead, let's restructure: if there's no second (), it's just params
                return self.finish_pattern_decl_simple(
                    attrs, vis, name, name_span, generics, first_params.into_iter().collect(), start_pos,
                );
            }
        } else {
            List::new()
        };

        // Pattern parameters (the value(s) being matched against)
        let params = self.parse_function_params()?;

        self.finish_pattern_decl_simple(attrs, vis, name, name_span, generics, params.into_iter().collect(), start_pos)
    }

    /// Finish parsing a pattern declaration after parameters are parsed.
    fn finish_pattern_decl_simple(
        &mut self,
        attrs: Vec<Attribute>,
        vis: Visibility,
        name: Text,
        name_span: Span,
        generics: List<GenericParam>,
        params: List<FunctionParam>,
        start_pos: usize,
    ) -> ParseResult<Item> {
        // Return type: -> Type
        self.stream.expect(TokenKind::RArrow)?;
        let return_type = self.parse_type()?;

        // Body: = expression
        self.stream.expect(TokenKind::Eq)?;
        let body = self.parse_expr()?;

        // Semicolon
        self.stream.expect(TokenKind::Semicolon)?;

        let span = self.stream.make_span(start_pos);
        let mut decl = PatternDecl::new(
            Ident::new(name, name_span),
            params,
            return_type,
            body,
            span,
        );
        decl.generics = generics;
        decl.visibility = vis;
        decl.attributes = attrs.into_iter().collect();

        Ok(Item::new(ItemKind::Pattern(decl), span))
    }
}

/// recursive depth probe for
/// HIT path-constructor endpoints.
///
/// A 1-cell path-constructor's endpoints are bare expressions (`Var`,
/// `App`, …) — depth 0. An n-cell endpoint is itself a parenthesised
/// range `(a..b)` — depth = 1 + max-depth-of-inner-endpoints. The
/// variant parser uses this to compute `path_dim`:
///
///   `dim = max(depth(lhs), depth(rhs)) + 1`
///
/// (Adding 1 because the outer `..` itself is one dimensional step.)
///
/// Examples (depth in `()`s):
///   * `Base` → depth 0
///   * `(loop_a..loop_b)` (`Paren` of `Range`) → depth 1
///   * `((p..q)..(r..s))` → depth 2
///
/// Non-path expressions return depth 0 unconditionally — the parser
/// treats anything that isn't a `Paren(Range(_, _))` as a point
/// endpoint.
fn path_endpoint_depth(expr: &verum_ast::Expr) -> u32 {
    use verum_ast::ExprKind;
    use verum_common::Maybe;
    match &expr.kind {
        ExprKind::Paren(inner) => match &inner.kind {
            ExprKind::Range {
                start: Maybe::Some(lhs),
                end: Maybe::Some(rhs),
                ..
            } => 1 + path_endpoint_depth(lhs).max(path_endpoint_depth(rhs)),
            _ => path_endpoint_depth(inner),
        },
        ExprKind::Range {
            start: Maybe::Some(lhs),
            end: Maybe::Some(rhs),
            ..
        } => 1 + path_endpoint_depth(lhs).max(path_endpoint_depth(rhs)),
        _ => 0,
    }
}

#[cfg(test)]
mod path_endpoint_depth_tests {
    use super::path_endpoint_depth;
    use verum_ast::{Expr, ExprKind, Path};
    use verum_ast::span::Span;
    use verum_common::{Heap, Maybe};

    fn var(name: &str) -> Expr {
        let span = Span::default();
        Expr::path(Path::single(verum_ast::Ident::new(verum_common::Text::from(name), span)))
    }

    fn range(lhs: Expr, rhs: Expr) -> Expr {
        Expr::new(
            ExprKind::Range {
                start: Maybe::Some(Heap::new(lhs)),
                end: Maybe::Some(Heap::new(rhs)),
                inclusive: false,
            },
            Span::default(),
        )
    }

    fn paren(inner: Expr) -> Expr {
        Expr::new(ExprKind::Paren(Heap::new(inner)), Span::default())
    }

    #[test]
    fn point_endpoint_has_depth_zero() {
        assert_eq!(path_endpoint_depth(&var("Base")), 0);
    }

    #[test]
    fn paren_around_var_has_depth_zero() {
        // `(Base)` — paren around non-range → 0.
        assert_eq!(path_endpoint_depth(&paren(var("Base"))), 0);
    }

    #[test]
    fn paren_around_range_is_one_cell() {
        // `(a..b)` is a 1-cell endpoint — depth 1.
        let p = paren(range(var("a"), var("b")));
        assert_eq!(path_endpoint_depth(&p), 1);
    }

    #[test]
    fn nested_paren_range_is_two_cell() {
        // `((p..q)..(r..s))` is a 2-cell endpoint — depth 2.
        let inner_lhs = paren(range(var("p"), var("q")));
        let inner_rhs = paren(range(var("r"), var("s")));
        let p = paren(range(inner_lhs, inner_rhs));
        assert_eq!(path_endpoint_depth(&p), 2);
    }

    #[test]
    fn three_deep_nesting_yields_depth_three() {
        let l1 = paren(range(var("a"), var("b")));
        let l1b = paren(range(var("c"), var("d")));
        let l2 = paren(range(l1, l1b));
        let l1c = paren(range(var("e"), var("f")));
        let l1d = paren(range(var("g"), var("h")));
        let l2b = paren(range(l1c, l1d));
        let l3 = paren(range(l2, l2b));
        assert_eq!(path_endpoint_depth(&l3), 3);
    }
}
