//! Proof parsing for Verum formal proofs system.
//!
//! This module implements parsing for:
//! - Theorem, axiom, lemma, corollary declarations
//! - Proof bodies (term, tactic, structured, by-method)
//! - Tactic expressions (35+ tactics)
//! - Calculation chains (equational reasoning)
//! - Proof methods (induction, cases, contradiction)
//!
//! Verum formal proofs (v2.0+ extension) enable machine-checkable mathematical
//! proofs. Theorems/lemmas/corollaries use syntax: `theorem name(params): proposition { proof }`.
//! Proof bodies can be term-mode, tactic-mode (`proof { ... }`), structured, or `by method`.
//! Tactics include: simp (simplify), ring (normalize ring exprs), omega (linear arithmetic),
//! induction, cases, contradiction, calc chains. Proofs are first-class values of type Proof<P>.

use verum_ast::decl::{
    AxiomDecl, CalcRelation, CalculationChain, CalculationStep, ProofBody, ProofCase, ProofMethod,
    ProofStep, ProofStepKind, ProofStructure, TacticBody, TacticDecl, TacticExpr, TacticParam,
    TacticParamKind, TheoremDecl,
};
use verum_ast::ty::{GenericArg, GenericParam, WhereClause, WherePredicateKind};
use verum_ast::{Expr, ExprKind, Ident, Item, ItemKind, Pattern, Span, Type};
use verum_common::{Heap, List, Maybe, Text};
use verum_lexer::TokenKind;

use crate::error::ParseError;
use crate::parser::{ParseResult, RecursiveParser};

impl<'a> RecursiveParser<'a> {
    // ============================================================================
    // Ghost Attribute Helper
    // ============================================================================

    /// Skip an optional `@ghost` prefix before an expected keyword token.
    /// This allows `@ghost requires ...` and `@ghost ensures ...` in contracts.
    /// Uses lookahead to verify the pattern before consuming tokens.
    pub(crate) fn skip_ghost_prefix_before(&mut self, expected_next: &TokenKind) {
        // Check pattern: @ ghost <expected_next> using lookahead
        if !self.stream.check(&TokenKind::At) {
            return;
        }
        // peek_nth(1) = "ghost" ident?
        let is_ghost = matches!(
            self.stream.peek_nth_kind(1),
            Some(TokenKind::Ident(name)) if name.as_str() == "ghost"
        );
        if !is_ghost {
            return;
        }
        // peek_nth(2) = expected keyword?
        if self.stream.peek_nth_kind(2) != Some(expected_next) {
            return;
        }
        // Pattern matches — consume `@` and `ghost`
        self.stream.advance(); // @
        self.stream.advance(); // ghost
    }

    // ============================================================================
    // Theorem Declarations
    // ============================================================================

    /// Parse a theorem declaration.
    /// Syntax: [pub] theorem name<T>(params): proposition { proof }
    ///
    /// Theorem syntax: `[pub] theorem name<T>(params): proposition { proof_body }`
    /// Propositions are type expressions. Proof bodies are term/tactic/structured/by-method.
    pub fn parse_theorem(
        &mut self,
        attrs: Vec<verum_ast::attr::Attribute>,
        vis: verum_ast::decl::Visibility,
    ) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // theorem keyword
        self.stream.expect(TokenKind::Theorem)?;

        self.parse_theorem_like(attrs, vis, ItemKind::Theorem, start_pos)
    }

    /// Parse a proof declaration.
    /// Syntax: [pub] proof name<T>(params): proposition { proof_body }
    /// Treated as a theorem-like declaration.
    pub fn parse_proof_decl(
        &mut self,
        attrs: Vec<verum_ast::attr::Attribute>,
        vis: verum_ast::decl::Visibility,
    ) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // proof keyword
        self.stream.expect(TokenKind::Proof)?;

        self.parse_theorem_like(attrs, vis, ItemKind::Theorem, start_pos)
    }

    /// Parse a lemma declaration.
    /// Syntax: [pub] lemma name<T>(params): proposition { proof }
    ///
    /// Lemma syntax: `[pub] lemma name<T>(params): proposition { proof_body }`
    /// Same structure as theorem but indicates a helper result.
    pub fn parse_lemma(
        &mut self,
        attrs: Vec<verum_ast::attr::Attribute>,
        vis: verum_ast::decl::Visibility,
    ) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // lemma keyword
        self.stream.expect(TokenKind::Lemma)?;

        // E022: Check for empty lemma (semicolon right after keyword)
        if self.stream.check(&TokenKind::Semicolon) {
            return Err(ParseError::invalid_lemma(
                "lemma declaration requires a name",
                self.stream.current_span(),
            ));
        }

        self.parse_theorem_like(attrs, vis, ItemKind::Lemma, start_pos)
    }

    /// Parse a corollary declaration.
    /// Syntax: [pub] corollary name<T>(params): proposition { proof }
    ///
    /// Corollary syntax: `[pub] corollary name<T>(params): proposition { proof_body }`
    /// A consequence derived from a previously proven theorem.
    pub fn parse_corollary(
        &mut self,
        attrs: Vec<verum_ast::attr::Attribute>,
        vis: verum_ast::decl::Visibility,
    ) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // corollary keyword
        self.stream.expect(TokenKind::Corollary)?;

        self.parse_theorem_like(attrs, vis, ItemKind::Corollary, start_pos)
    }

    /// Parse theorem-like declarations (theorem, lemma, corollary).
    ///
    /// Supports two syntax forms:
    /// 1. Contract-based: `theorem name(params) requires ... ensures ... { proof by tactic }`
    /// 2. Proposition-based: `theorem name(params): proposition { tactic }`
    fn parse_theorem_like(
        &mut self,
        attrs: Vec<verum_ast::attr::Attribute>,
        vis: verum_ast::decl::Visibility,
        kind_fn: fn(TheoremDecl) -> ItemKind,
        start_pos: usize,
    ) -> ParseResult<Item> {
        // E020/E022: Check for empty theorem/lemma (semicolon right after keyword)
        if self.stream.check(&TokenKind::Semicolon) {
            return Err(ParseError::invalid_theorem(
                "theorem/lemma declaration requires a name",
                self.stream.current_span(),
            ));
        }

        // E021: Check for missing name (keyword like 'is' instead of identifier)
        if self.stream.check(&TokenKind::Is) {
            return Err(ParseError::missing_theorem_name(
                self.stream.current_span(),
            ));
        }

        // Theorem name — accept contextual keywords (e.g., `trivial`)
        // as theorem identifiers so that `theorem trivial() proof by
        // trivial;` is valid.
        let name = self.consume_ident_or_any_keyword()?;
        let name_span = self.stream.current_span();

        // Optional generic parameters: <T, U>
        let generics: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?
        } else {
            List::new()
        };

        // Optional parameters: (x: Nat, y: Nat)
        let params = if self.stream.check(&TokenKind::LParen) {
            self.parse_function_params()?
        } else {
            Vec::new()
        };

        // Check for optional return type: -> Type
        // Use parse_type_no_sigma to prevent `-> Bool: n` from being parsed as sigma type
        // In theorem/lemma, the `:` after the type introduces the proposition, not a sigma constraint
        let return_type = if self.stream.consume(&TokenKind::RArrow).is_some() {
            Maybe::Some(self.parse_type_no_sigma()?)
        } else {
            Maybe::None
        };

        // Determine syntax form based on next token
        let (requires, ensures, proposition, generic_where, meta_where, proof) =
            if self.stream.check(&TokenKind::Colon) {
                // Proposition-based syntax: `: proposition { tactic }`
                self.stream.advance();
                // Use parse_expr_no_struct to prevent `{` from being consumed as a record literal
                let prop = self.parse_expr_no_struct()?;

                // Optional where clause
                let (gw, mw) = if self.stream.check(&TokenKind::Where) {
                    let where_clause = self.parse_where_clause()?;
                    self.separate_where_clauses(&where_clause)
                } else {
                    (Maybe::None, Maybe::None)
                };

                // Optional proof body: { tactic } or proof { ... } or proof by tactic or ;
                let pf = if self.stream.check(&TokenKind::LBrace) {
                    let body = self.parse_proof_body()?;
                    self.stream.consume(&TokenKind::Semicolon);
                    Maybe::Some(body)
                } else if self.stream.check(&TokenKind::Proof) {
                    let body = self.parse_proof_body()?;
                    self.stream.consume(&TokenKind::Semicolon);
                    Maybe::Some(body)
                } else {
                    self.stream.consume(&TokenKind::Semicolon);
                    Maybe::None
                };

                (List::new(), List::new(), prop, gw, mw, pf)
            } else {
                // Contract-based syntax: `requires ... ensures ... { proof ... }`

                // Parse requires/given/ensures clauses in any order
                let mut reqs = Vec::new();
                let mut enss = Vec::new();
                loop {
                    // Skip bare `@attr` / `@attr(args)` attributes sprinkled
                    // between the parameter list and the contract clauses —
                    // the stdlib (e.g. `core/math/day_convolution.vr`) writes
                    //     theorem foo(…)
                    //         @verify(formal)
                    //         ensures …
                    //         proof by auto;
                    // The attribute is a documentation/tactic hint here, not
                    // a decl attribute, so we discard it rather than fail.
                    if self.stream.check(&TokenKind::At) {
                        self.stream.advance(); // consume `@`
                        // Consume identifier name (attribute name).
                        if self.is_ident() {
                            let _ = self.consume_ident_or_any_keyword();
                        }
                        // Optional argument list `(…)` — track paren depth.
                        if self.stream.check(&TokenKind::LParen) {
                            let mut depth = 0_i32;
                            loop {
                                if !self.tick() || self.is_aborted() { break; }
                                match self.stream.peek_kind() {
                                    Some(TokenKind::LParen) => { depth += 1; self.stream.advance(); }
                                    Some(TokenKind::RParen) => {
                                        depth -= 1;
                                        self.stream.advance();
                                        if depth == 0 { break; }
                                    }
                                    None => break,
                                    _ => { self.stream.advance(); }
                                }
                            }
                        }
                        continue;
                    }
                    // requires clause
                    self.skip_ghost_prefix_before(&TokenKind::Requires);
                    if self.stream.consume(&TokenKind::Requires).is_some() {
                        reqs.extend(self.parse_contract_expr_list()?);
                        continue;
                    }
                    // given clause: `given name: Type` - treat as named hypothesis (like requires)
                    if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("given"))) {
                        self.stream.advance(); // consume "given"
                        // Parse `name: expr` as a requires
                        let _name = self.consume_ident()?;
                        if self.stream.consume(&TokenKind::Colon).is_some() {
                            let expr = self.parse_expr_no_struct()?;
                            reqs.push(expr);
                        }
                        continue;
                    }
                    // ensures clause
                    self.skip_ghost_prefix_before(&TokenKind::Ensures);
                    if self.stream.consume(&TokenKind::Ensures).is_some() {
                        enss.extend(self.parse_contract_expr_list()?);
                        continue;
                    }
                    // `from <ident>` clause — provenance marker used by
                    // corollaries to name the theorem they follow from.
                    // Parsed and discarded at the parser level; the trace
                    // is preserved in documentation and the SMT backend
                    // re-derives the obligation independently.
                    if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("from"))) {
                        self.stream.advance();
                        let _parent = self.consume_ident()?;
                        continue;
                    }
                    break;
                }

                // Check for explicit proposition with `:` syntax after requires/ensures
                // This allows: theorem T(x: Int) requires x > 0 : x + 0 == x { ... }
                let prop = if self.stream.consume(&TokenKind::Colon).is_some() {
                    // Explicit proposition syntax after contracts
                    self.parse_expr_no_struct()?
                } else if !enss.is_empty() {
                    // Combine all ensures with logical AND
                    self.make_conjunction(&enss)
                } else {
                    // Default to true literal
                    Expr::new(
                        ExprKind::Literal(verum_ast::Literal::bool(true, self.stream.current_span())),
                        self.stream.current_span(),
                    )
                };

                // Optional where clause
                let (gw, mw) = if self.stream.check(&TokenKind::Where) {
                    let where_clause = self.parse_where_clause()?;
                    self.separate_where_clauses(&where_clause)
                } else {
                    (Maybe::None, Maybe::None)
                };

                // Proof body: { proof ... } or proof { ... } or proof by tactic or ;
                let pf = if self.stream.check(&TokenKind::LBrace) {
                    let body = self.parse_proof_body()?;
                    self.stream.consume(&TokenKind::Semicolon);
                    Maybe::Some(body)
                } else if self.stream.check(&TokenKind::Proof) {
                    // Standalone proof keyword - parse it
                    let body = self.parse_proof_body()?;
                    self.stream.consume(&TokenKind::Semicolon);
                    Maybe::Some(body)
                } else {
                    self.stream.consume(&TokenKind::Semicolon);
                    Maybe::None
                };

                (reqs.into_iter().collect(), enss.into_iter().collect(), prop, gw, mw, pf)
            };

        let span = self.stream.make_span(start_pos);
        let decl = TheoremDecl {
            visibility: vis,
            name: Ident::new(name, name_span),
            generics,
            params: params.into_iter().collect(),
            return_type,
            requires,
            ensures,
            proposition: Heap::new(proposition),
            generic_where_clause: generic_where,
            meta_where_clause: meta_where,
            proof,
            attributes: attrs.into_iter().collect(),
            span,
        };

        Ok(Item::new(kind_fn(decl), span))
    }

    /// Parse a list of contract expressions separated by commas.
    /// Uses parse_expr_no_struct to prevent `{` from being consumed as a struct literal.
    /// Stops at keywords like `ensures`, `where`, `{`, or `;`.
    fn parse_contract_expr_list(&mut self) -> ParseResult<Vec<Expr>> {
        let mut exprs = Vec::new();

        // Parse first expression - use parse_expr_no_struct to avoid consuming `{`
        exprs.push(self.parse_expr_no_struct()?);

        // Parse additional comma-separated expressions
        while self.stream.consume(&TokenKind::Comma).is_some() {
            // Stop if we hit a keyword that starts a new clause
            if self.stream.check(&TokenKind::Ensures)
                || self.stream.check(&TokenKind::Requires)
                || self.stream.check(&TokenKind::Where)
                || self.stream.check(&TokenKind::LBrace)
            {
                break;
            }
            exprs.push(self.parse_expr_no_struct()?);
        }

        Ok(exprs)
    }

    /// Create a conjunction (AND) of expressions.
    fn make_conjunction(&self, exprs: &[Expr]) -> Expr {
        if exprs.is_empty() {
            Expr::new(
                ExprKind::Literal(verum_ast::Literal::bool(true, Span::default())),
                Span::default(),
            )
        } else if exprs.len() == 1 {
            exprs[0].clone()
        } else {
            // Combine with logical AND
            let mut result = exprs[0].clone();
            for expr in &exprs[1..] {
                // Use the span from the first expression, extending to the end of the last
                let span = Span::new(result.span.start, expr.span.end, result.span.file_id);
                result = Expr::new(
                    ExprKind::Binary {
                        op: verum_ast::BinOp::And,
                        left: Heap::new(result),
                        right: Heap::new(expr.clone()),
                    },
                    span,
                );
            }
            result
        }
    }

    /// Separate generic and meta predicates from a where clause.
    fn separate_where_clauses(
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

    // ============================================================================
    // Axiom Declarations
    // ============================================================================

    /// Parse an axiom declaration.
    /// Syntax: [pub] axiom name<T>(params): proposition;
    ///
    /// Axiom syntax: `[pub] axiom name<T>(params): proposition;`
    /// Axioms are unproven assumptions — accepted as true without proof body.
    pub fn parse_axiom(
        &mut self,
        attrs: Vec<verum_ast::attr::Attribute>,
        vis: verum_ast::decl::Visibility,
    ) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // axiom keyword
        self.stream.expect(TokenKind::Axiom)?;

        // Axiom name
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        // Optional generic parameters
        let generics: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?
        } else {
            List::new()
        };

        // Optional parameters (may be omitted for zero-arg axioms)
        let params = if self.stream.check(&TokenKind::LParen) {
            self.parse_function_params()?
        } else {
            Vec::new()
        };

        // Optional return type: -> Type
        // Use parse_type_no_sigma to prevent `-> Bool: proposition` from being parsed as sigma type
        let return_type = if self.stream.consume(&TokenKind::RArrow).is_some() {
            Maybe::Some(self.parse_type_no_sigma()?)
        } else {
            Maybe::None
        };

        // Optional [@ghost] requires and ensures clauses (in any order).
        //
        // Semantics in model theory: an axiom is a universally-quantified
        // proposition over its parameters. `requires R ensures E` states
        // `forall params. R → E`. `ensures E` alone states `forall params. E`.
        // `requires R` alone is legacy and gets interpreted as the asserted
        // proposition (for backwards compatibility with pre-T1-Q axioms).
        let mut requires_clauses: Vec<Expr> = Vec::new();
        let mut ensures_clauses: Vec<Expr> = Vec::new();
        loop {
            self.skip_ghost_prefix_before(&TokenKind::Requires);
            self.skip_ghost_prefix_before(&TokenKind::Ensures);
            if self.stream.consume(&TokenKind::Requires).is_some() {
                let expr = self.parse_expr_bp(4)?;
                requires_clauses.push(expr);
                while self.stream.consume(&TokenKind::Comma).is_some() {
                    if self.stream.check(&TokenKind::Colon)
                        || self.stream.check(&TokenKind::Semicolon)
                        || self.stream.check(&TokenKind::Where)
                        || self.stream.check(&TokenKind::Requires)
                        || self.stream.check(&TokenKind::Ensures)
                    {
                        break;
                    }
                    let expr = self.parse_expr_bp(4)?;
                    requires_clauses.push(expr);
                }
            } else if self.stream.consume(&TokenKind::Ensures).is_some() {
                let expr = self.parse_expr_bp(4)?;
                ensures_clauses.push(expr);
                while self.stream.consume(&TokenKind::Comma).is_some() {
                    if self.stream.check(&TokenKind::Colon)
                        || self.stream.check(&TokenKind::Semicolon)
                        || self.stream.check(&TokenKind::Where)
                        || self.stream.check(&TokenKind::Requires)
                        || self.stream.check(&TokenKind::Ensures)
                    {
                        break;
                    }
                    let expr = self.parse_expr_bp(4)?;
                    ensures_clauses.push(expr);
                }
            } else {
                break;
            }
        }

        // Optional proposition after colon: `: assertion` — explicit form.
        // Otherwise synthesize the proposition from the clauses:
        //   * with ensures E₁, …, Eₙ      → E₁ ∧ … ∧ Eₙ   (the canonical form)
        //   * with requires R₁, …, Rₖ     → R₁ ∧ … ∧ Rₖ   (legacy, pre-T1-Q)
        //   * with both                   → (R₁ ∧ …) → (E₁ ∧ …)
        //   * neither                     → `true`
        let proposition = if self.stream.consume(&TokenKind::Colon).is_some() {
            self.parse_expr()?
        } else if !ensures_clauses.is_empty() {
            let conj = |mut clauses: Vec<Expr>| {
                let first = clauses.remove(0);
                clauses.into_iter().fold(first, |acc, rhs| {
                    let span = Span::new(acc.span.start, rhs.span.end, acc.span.file_id);
                    Expr::new(
                        ExprKind::Binary {
                            op: verum_ast::BinOp::And,
                            left: Heap::new(acc),
                            right: Heap::new(rhs),
                        },
                        span,
                    )
                })
            };
            let ensures_body = conj(ensures_clauses);
            if requires_clauses.is_empty() {
                ensures_body
            } else {
                let requires_body = conj(requires_clauses);
                // requires → ensures  ≡  !requires || ensures
                let span = Span::new(
                    requires_body.span.start,
                    ensures_body.span.end,
                    requires_body.span.file_id,
                );
                let neg_req = Expr::new(
                    ExprKind::Unary {
                        op: verum_ast::expr::UnOp::Not,
                        expr: Heap::new(requires_body),
                    },
                    span,
                );
                Expr::new(
                    ExprKind::Binary {
                        op: verum_ast::BinOp::Or,
                        left: Heap::new(neg_req),
                        right: Heap::new(ensures_body),
                    },
                    span,
                )
            }
        } else if !requires_clauses.is_empty() {
            // Legacy form: requires-only means "the axiom asserts these hold"
            let first = requires_clauses.remove(0);
            requires_clauses.into_iter().fold(first, |acc, req| {
                let span = Span::new(acc.span.start, req.span.end, acc.span.file_id);
                Expr::new(
                    ExprKind::Binary {
                        op: verum_ast::BinOp::And,
                        left: Heap::new(acc),
                        right: Heap::new(req),
                    },
                    span,
                )
            })
        } else {
            // No explicit proposition - create a `true` literal
            let span = self.stream.current_span();
            Expr::new(ExprKind::Literal(verum_ast::Literal::bool(true, span)), span)
        };

        // Optional where clause
        let (generic_where, meta_where) = if self.stream.check(&TokenKind::Where) {
            let where_clause = self.parse_where_clause()?;
            self.separate_where_clauses(&where_clause)
        } else {
            (Maybe::None, Maybe::None)
        };

        // Axioms don't have proof bodies - semicolon or nothing
        self.stream.consume(&TokenKind::Semicolon);

        let span = self.stream.make_span(start_pos);
        let decl = AxiomDecl {
            visibility: vis,
            name: Ident::new(name, name_span),
            generics,
            params: params.into_iter().collect(),
            return_type,
            proposition: Heap::new(proposition),
            generic_where_clause: generic_where,
            meta_where_clause: meta_where,
            attributes: attrs.into_iter().collect(),
            span,
        };

        Ok(Item::new(ItemKind::Axiom(decl), span))
    }

    // ============================================================================
    // Tactic Declarations
    // ============================================================================

    /// Parse a tactic declaration.
    /// Syntax: tactic name(params) { body }
    ///
    /// Grammar: tactic_decl = 'tactic' , identifier , '(' , [ param_list ] , ')' , block_expr ;
    ///
    /// Tactic declarations define reusable proof strategies. Built-in tactics include:
    /// simp (simplify), ring (normalize ring exprs), field (normalize field exprs),
    /// omega (linear arithmetic), blast (tableau prover). User tactics compose these.
    pub fn parse_tactic_decl(&mut self, vis: verum_ast::decl::Visibility) -> ParseResult<Item> {
        let start_pos = self.stream.position();

        // Check for 'tactic' as identifier (contextual keyword)
        match self.stream.peek_kind() {
            Some(TokenKind::Ident(name)) if name.as_str() == "tactic" => {
                self.stream.advance();
            }
            _ => {
                return Err(ParseError::invalid_syntax(
                    "expected 'tactic' keyword",
                    self.stream.current_span(),
                ));
            }
        }

        // Tactic name. Built-in tactic keywords (`assumption`, `contradiction`,
        // `trivial`, `simp`, `ring`, `field`, `omega`, `blast`, `smt`,
        // `induction`, `cases`, `auto`, `proof`, `lemma`, `theorem`,
        // `axiom`, `corollary`, `match`, `return`) can legitimately appear
        // as user-tactic names — they override the built-in implementation
        // within the declaring module. Accept them verbatim here so the
        // stdlib can declare, e.g. `tactic assumption() { ... }`.
        let name_span = self.stream.current_span();
        let name = match self.stream.peek_kind() {
            Some(TokenKind::Assumption) => { self.stream.advance(); Text::from("assumption") }
            Some(TokenKind::Contradiction) => { self.stream.advance(); Text::from("contradiction") }
            Some(TokenKind::Trivial) => { self.stream.advance(); Text::from("trivial") }
            Some(TokenKind::Simp) => { self.stream.advance(); Text::from("simp") }
            Some(TokenKind::Ring) => { self.stream.advance(); Text::from("ring") }
            Some(TokenKind::Field) => { self.stream.advance(); Text::from("field") }
            Some(TokenKind::Omega) => { self.stream.advance(); Text::from("omega") }
            Some(TokenKind::Blast) => { self.stream.advance(); Text::from("blast") }
            Some(TokenKind::Smt) => { self.stream.advance(); Text::from("smt") }
            Some(TokenKind::Induction) => { self.stream.advance(); Text::from("induction") }
            Some(TokenKind::Cases) => { self.stream.advance(); Text::from("cases") }
            Some(TokenKind::Auto) => { self.stream.advance(); Text::from("auto") }
            Some(TokenKind::Proof) => { self.stream.advance(); Text::from("proof") }
            Some(TokenKind::Lemma) => { self.stream.advance(); Text::from("lemma") }
            Some(TokenKind::Theorem) => { self.stream.advance(); Text::from("theorem") }
            Some(TokenKind::Axiom) => { self.stream.advance(); Text::from("axiom") }
            Some(TokenKind::Corollary) => { self.stream.advance(); Text::from("corollary") }
            // `quote` is reserved for staged-metaprogramming expressions in
            // `fn` bodies, but inside a `tactic` decl the stdlib uses it as
            // the surface name of the meta-programming primitive — same
            // override pattern as the other reserved tactic keywords above.
            Some(TokenKind::QuoteKeyword) => { self.stream.advance(); Text::from("quote") }
            _ => self.consume_ident()?,
        };

        // Optional generic parameters: tactic category_law<C: Category>()
        // Grammar §2.19.5 (industrial-grade extension over the minimal EBNF
        // rule `tactic_decl = 'tactic' identifier '(' ... ')' block_expr`):
        // tactics are polymorphic, parameterised by types or protocol bounds.
        let generics: List<GenericParam> = if self.stream.check(&TokenKind::Lt) {
            self.parse_generic_params()?
        } else {
            List::new()
        };

        // Optional parameters
        let params = if self.stream.check(&TokenKind::LParen) {
            self.parse_tactic_params()?
        } else {
            Vec::new()
        };

        // Optional `where` clause on generics (e.g. `where C: Category + HasId`)
        let generic_where_clause = if self
            .stream
            .peek_kind()
            .map(|k| matches!(k, TokenKind::Ident(n) if n.as_str() == "where"))
            .unwrap_or(false)
        {
            Maybe::Some(self.parse_where_clause()?)
        } else {
            Maybe::None
        };

        // Tactic body
        let body = self.parse_tactic_body()?;

        let span = self.stream.make_span(start_pos);
        let decl = TacticDecl {
            visibility: vis,
            name: Ident::new(name, name_span),
            generics,
            params: params.into_iter().collect(),
            generic_where_clause,
            body,
            attributes: List::new(),
            span,
        };

        Ok(Item::new(ItemKind::Tactic(decl), span))
    }

    /// Parse tactic parameters: (x: Expr, t: Tactic)
    fn parse_tactic_params(&mut self) -> ParseResult<Vec<TacticParam>> {
        let open_span = self.stream.current_span();
        self.stream.expect(TokenKind::LParen)?;

        // E028: Check for malformed tactic - unexpected closing brace
        if self.stream.check(&TokenKind::RBrace) {
            return Err(ParseError::malformed_tactic(
                "unclosed tactic parameter list, expected ')' before '}'",
                open_span,
            ));
        }

        let params = if self.stream.check(&TokenKind::RParen) {
            Vec::new()
        } else {
            // Check for EOF in parameter list
            if self.stream.at_end() {
                return Err(ParseError::malformed_tactic(
                    "unclosed tactic parameter list",
                    open_span,
                ));
            }
            self.comma_separated(|p| p.parse_tactic_param())?
        };

        // E028: Check for unclosed parameter list
        if !self.stream.check(&TokenKind::RParen) {
            return Err(ParseError::malformed_tactic(
                "unclosed tactic parameter list, expected ')'",
                self.stream.current_span(),
            ));
        }
        self.stream.advance(); // consume )
        Ok(params)
    }

    /// Parse a single tactic parameter.
    ///
    /// Tactic parameters are typed bindings — either one of the classical
    /// tactic-DSL kinds (`Expr`, `Type`, `Tactic`, `Hypothesis`, `Int`,
    /// `Prop`) or an arbitrary concrete type (`Float`, `List<T>`,
    /// `Maybe<Proof>`, …). An optional default value can be provided with
    /// `= expr` so tactic authors can write `confidence: Float = 0.9`.
    fn parse_tactic_param(&mut self) -> ParseResult<TacticParam> {
        let start_pos = self.stream.position();

        // Parameter name
        let name = self.consume_ident()?;
        let name_span = self.stream.current_span();

        // Colon
        self.stream.expect(TokenKind::Colon)?;

        // Peek the next identifier to decide whether we are looking at a
        // classical kind or a full type expression. We still accept the
        // classical kinds verbatim so `expr`, `tactic`, `hypothesis`, `int`,
        // `Prop` declarations keep their historic semantics — but anything
        // else (including `Float`, `List<A>`, `Maybe<Proof>`, …) is parsed as
        // a real `Type` and stored in `ty` with `kind = Other`.
        let (kind, ty) = match self.stream.peek_kind() {
            Some(TokenKind::Ident(name))
                if matches!(
                    name.as_str(),
                    "Expr" | "expr"
                        | "Type"
                        | "type"
                        | "Tactic"
                        | "tactic"
                        | "Hypothesis"
                        | "hypothesis"
                        | "H"
                        | "Int"
                        | "int"
                        | "Prop"
                        | "prop"
                )
                // The classical kinds only match when they are *not* followed
                // by `<…>` or `.` — otherwise they are the head of a real
                // type like `Int.Max` or `Prop<A>`.
                && !self.peek_is_type_tail_after_ident() =>
            {
                let kind_name = self.consume_ident()?;
                let kind = match kind_name.as_str() {
                    "Expr" | "expr" => TacticParamKind::Expr,
                    "Type" | "type" => TacticParamKind::Type,
                    "Tactic" | "tactic" => TacticParamKind::Tactic,
                    "Hypothesis" | "hypothesis" | "H" => TacticParamKind::Hypothesis,
                    "Int" | "int" => TacticParamKind::Int,
                    "Prop" | "prop" => TacticParamKind::Prop,
                    _ => unreachable!(),
                };
                (kind, Maybe::None)
            }
            _ => {
                // Full type expression — anything else (including user types).
                let ty = self.parse_type()?;
                (TacticParamKind::Other, Maybe::Some(ty))
            }
        };

        // Optional default value: `= expr`
        let default = if self.stream.consume(&TokenKind::Eq).is_some() {
            Maybe::Some(Heap::new(self.parse_expr()?))
        } else {
            Maybe::None
        };

        let span = self.stream.make_span(start_pos);
        Ok(TacticParam {
            name: Ident::new(name, name_span),
            kind,
            ty,
            default,
            span,
        })
    }

    /// Peek past the current identifier to decide whether it is the head of a
    /// composite type expression (`Int.Max`, `Prop<A>`, `Int[N]`, `Int &`…).
    /// Used to disambiguate classical tactic-param kinds from real types with
    /// matching names.
    fn peek_is_type_tail_after_ident(&self) -> bool {
        matches!(
            self.stream.peek_nth_kind(1),
            Some(TokenKind::Dot)
                | Some(TokenKind::Lt)
                | Some(TokenKind::LBracket)
                | Some(TokenKind::Ampersand)
                | Some(TokenKind::ColonColon)
        )
    }

    /// Parse tactic body.
    fn parse_tactic_body(&mut self) -> ParseResult<TacticBody> {
        if self.stream.check(&TokenKind::LBrace) {
            // Block body: { tactic1; tactic2 }
            self.stream.advance();
            let mut tactics = Vec::new();

            while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
                let tactic = self.parse_tactic_expr()?;
                tactics.push(tactic);

                // Optional semicolon between tactics
                self.stream.consume(&TokenKind::Semicolon);
            }

            self.stream.expect(TokenKind::RBrace)?;
            Ok(TacticBody::Block(tactics.into_iter().collect()))
        } else {
            // Simple body: just a tactic expression
            let tactic = self.parse_tactic_expr()?;
            Ok(TacticBody::Simple(tactic))
        }
    }

    // ============================================================================
    // Proof Bodies
    // ============================================================================

    /// Parse a proof body.
    /// Can be: { term }, proof { ... }, by method { ... }
    ///
    /// Proof bodies come in four forms:
    /// 1. Term-mode: direct expression `{ proof_term }`
    /// 2. Tactic-mode: `proof { have h1: ...; suffices ...; ... }`
    /// 3. By-method: `by induction on x { case ... }` / `by contradiction { ... }`
    /// 4. Structured: step-by-step with `have`, `suffices`, `show`, `cases`
    ///    Expect closing brace or accept at top-level keyword boundary.
    ///    Used in proof bodies where the closing `}` may be omitted when the proof
    ///    ends with a tactic and the next item is a top-level declaration.
    fn expect_rbrace_or_top_level(&mut self) -> ParseResult<()> {
        if self.stream.consume(&TokenKind::RBrace).is_some() {
            return Ok(());
        }
        // Accept if we're at a top-level keyword or EOF
        match self.stream.peek_kind() {
            Some(TokenKind::Theorem) | Some(TokenKind::Lemma) | Some(TokenKind::Axiom)
            | Some(TokenKind::Corollary) | Some(TokenKind::Fn) | Some(TokenKind::Type)
            | Some(TokenKind::Pub) | Some(TokenKind::At) | Some(TokenKind::Module)
            | Some(TokenKind::Mount) | Some(TokenKind::Const) | Some(TokenKind::Static)
            | Some(TokenKind::Implement) | None => Ok(()),
            _ => {
                self.stream.expect(TokenKind::RBrace)?;
                Ok(())
            }
        }
    }

    /// Skip a balanced brace block `{ ... }`, including nested braces.
    /// Used to skip optional configuration blocks on tactics.
    fn skip_balanced_braces(&mut self) {
        if self.stream.consume(&TokenKind::LBrace).is_none() {
            return;
        }
        let mut depth: u32 = 1;
        while depth > 0 && !self.stream.at_end() {
            if self.stream.check(&TokenKind::LBrace) {
                depth += 1;
            } else if self.stream.check(&TokenKind::RBrace) {
                depth -= 1;
                if depth == 0 {
                    self.stream.advance(); // consume closing brace
                    return;
                }
            }
            self.stream.advance();
        }
    }

    pub fn parse_proof_body(&mut self) -> ParseResult<ProofBody> {
        // Check for 'proof' keyword followed by block
        if self.stream.consume(&TokenKind::Proof).is_some() {
            return self.parse_structured_proof();
        }

        // Check for 'by' keyword (induction, cases, contradiction)
        if self.stream.check(&TokenKind::By) {
            return self.parse_proof_by_method();
        }

        // Otherwise it's a block - could be term proof or structured proof
        self.stream.expect(TokenKind::LBrace)?;

        // Look ahead to determine proof style
        match self.stream.peek_kind() {
            // Structured proof keywords
            Some(TokenKind::Have)
            | Some(TokenKind::Show)
            | Some(TokenKind::Suffices)
            | Some(TokenKind::Obtain)
            | Some(TokenKind::Calc)
            | Some(TokenKind::Cases)
            | Some(TokenKind::If)
            | Some(TokenKind::Let) => self.parse_structured_proof_body(),
            // Identifier-based proof steps: witness, conclude, assume, take, left, right
            Some(TokenKind::Ident(name))
                if matches!(
                    name.as_str(),
                    "take" | "witness" | "conclude" | "assume" | "left" | "right" | "Trivial" | "assumed"
                ) =>
            {
                self.parse_structured_proof_body()
            }
            // 'proof' keyword inside block: { proof by tactic } or { proof { ... } }
            Some(TokenKind::Proof) => {
                self.stream.advance(); // consume 'proof'
                let proof = self.parse_structured_proof()?;
                self.expect_rbrace_or_top_level()?;
                Ok(proof)
            }
            // By method inside block
            Some(TokenKind::By) => {
                let method = self.parse_proof_method_inner()?;
                self.stream.expect(TokenKind::RBrace)?;
                Ok(ProofBody::ByMethod(method))
            }
            // Tactic proof
            Some(TokenKind::Trivial)
            | Some(TokenKind::Assumption)
            | Some(TokenKind::Simp)
            | Some(TokenKind::Ring)
            | Some(TokenKind::Field)
            | Some(TokenKind::Omega)
            | Some(TokenKind::Auto)
            | Some(TokenKind::Blast)
            | Some(TokenKind::Smt)
            | Some(TokenKind::Induction) => {
                let tactic = self.parse_tactic_expr()?;
                self.expect_rbrace_or_top_level()?;
                Ok(ProofBody::Tactic(tactic))
            }
            // Default: term proof
            _ => {
                let expr = self.parse_expr()?;
                self.expect_rbrace_or_top_level()?;
                Ok(ProofBody::Term(Heap::new(expr)))
            }
        }
    }

    /// Parse structured proof body (already past opening brace).
    fn parse_structured_proof_body(&mut self) -> ParseResult<ProofBody> {
        let start_pos = self.stream.position();
        let mut steps = Vec::new();
        let conclusion = Maybe::None;

        while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
            // Check for QED marker
            if self.stream.consume(&TokenKind::Qed).is_some() {
                break;
            }

            // Parse proof step
            let step = self.parse_proof_step()?;
            steps.push(step);

            // Optional semicolon
            self.stream.consume(&TokenKind::Semicolon);
        }

        // E029: Check for unclosed proof block (hit EOF without closing brace)
        if self.stream.at_end() && !self.stream.check(&TokenKind::RBrace) {
            return Err(ParseError::proof_not_terminated(
                self.stream.make_span(start_pos),
            ));
        }

        self.stream.expect(TokenKind::RBrace)?;

        let span = self.stream.make_span(start_pos);
        Ok(ProofBody::Structured(ProofStructure {
            steps: steps.into_iter().collect(),
            conclusion,
            span,
        }))
    }

    /// Parse structured proof after 'proof' keyword.
    /// Handles: `proof by tactic`, `proof = term`, `proof { ... }`
    fn parse_structured_proof(&mut self) -> ParseResult<ProofBody> {
        // proof by method/tactic
        if self.stream.consume(&TokenKind::By).is_some() {
            // Check if this is a proof method (induction, cases, contradiction) that has a body
            // vs a simple tactic (simp, ring, omega, etc.)
            match self.stream.peek_kind() {
                Some(TokenKind::Induction) | Some(TokenKind::Cases) | Some(TokenKind::Contradiction) => {
                    let method = self.parse_proof_method_inner()?;
                    // Consume trailing tactic chain: `; omega; simp`
                    // These are additional tactics applied after the proof method.
                    // IMPORTANT: Only consume `;` if followed by a tactic keyword,
                    // not if `;` is the theorem terminator.
                    while self.stream.check(&TokenKind::Semicolon) {
                        // Look ahead: is the token after `;` a tactic expression?
                        let next = self.stream.peek_nth_kind(1);
                        let is_tactic_follow = matches!(next,
                            Some(TokenKind::Trivial)
                            | Some(TokenKind::Assumption)
                            | Some(TokenKind::Simp)
                            | Some(TokenKind::Ring)
                            | Some(TokenKind::Field)
                            | Some(TokenKind::Omega)
                            | Some(TokenKind::Auto)
                            | Some(TokenKind::Induction)
                            | Some(TokenKind::Cases)
                            | Some(TokenKind::Contradiction)
                            | Some(TokenKind::LParen)
                        ) || matches!(next, Some(TokenKind::Ident(n)) if matches!(n.as_str(),
                            "exact" | "apply" | "rewrite" | "unfold" | "intro" | "intros"
                            | "split" | "left" | "right" | "destruct" | "invert" | "specialize"
                            | "generalize" | "clear" | "rename" | "subst" | "symmetry"
                        ));
                        if !is_tactic_follow {
                            break;
                        }
                        self.stream.advance(); // consume `;`
                        if self.stream.check(&TokenKind::RBrace) || self.stream.at_end() {
                            break;
                        }
                        // Parse and discard additional tactic
                        let _tactic = self.parse_tactic_expr()?;
                    }
                    return Ok(ProofBody::ByMethod(method));
                }
                // Handle named proof methods like `strong_induction on n { ... }`
                // Also handles: well_founded_induction on (m, n) ordered_by ... { ... }
                //               mutual_induction on is_even, is_odd { ... }
                //               contrapositive { ... }
                Some(TokenKind::Ident(_)) => {
                    // Check if the identifier is followed by "on" (induction-like method)
                    // or directly by `{` (contradiction-like method)
                    let next1 = self.stream.peek_nth_kind(1);
                    let is_on = matches!(next1, Some(TokenKind::Ident(n)) if n.as_str() == "on");
                    let is_brace = matches!(next1, Some(TokenKind::LBrace));

                    if is_on {
                        // Pattern: <name> on <expr/ident> [ordered_by <expr>] { cases }
                        let _method_name = self.consume_ident()?;
                        self.stream.advance(); // consume "on"

                        // Parse the "on" target: could be ident, tuple, or comma-separated idents
                        let var = if self.stream.check(&TokenKind::LParen) {
                            // Tuple form: on (m, n)
                            let _tuple = self.parse_expr()?;
                            Ident::new(Text::from("_tuple"), self.stream.current_span())
                        } else {
                            Ident::new(self.consume_ident()?, self.stream.current_span())
                        };

                        // Consume additional comma-separated targets: on is_even, is_odd
                        while self.stream.consume(&TokenKind::Comma).is_some() {
                            if self.stream.check(&TokenKind::LBrace) || self.stream.at_end() {
                                break;
                            }
                            let _extra = self.consume_ident()?;
                        }

                        // Optional `ordered_by expr`
                        if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("ordered_by"))) {
                            self.stream.advance();
                            let _order = self.parse_expr_no_struct()?;
                        }

                        // Optional cases block
                        if self.stream.check(&TokenKind::LBrace) {
                            self.stream.advance();
                            let cases = self.parse_proof_cases()?;
                            self.stream.expect(TokenKind::RBrace)?;
                            return Ok(ProofBody::ByMethod(ProofMethod::Induction {
                                on: Maybe::Some(var),
                                cases: cases.into_iter().collect(),
                            }));
                        } else {
                            return Ok(ProofBody::ByMethod(ProofMethod::Induction {
                                on: Maybe::Some(var),
                                cases: List::new(),
                            }));
                        }
                    } else if is_brace {
                        // Pattern: <name> { ... } - like `contrapositive { ... }`
                        let _method_name = self.consume_ident()?;
                        self.stream.advance(); // consume {
                        let mut steps = Vec::new();
                        while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
                            let step = self.parse_proof_step()?;
                            steps.push(step);
                            self.stream.consume(&TokenKind::Semicolon);
                        }
                        self.stream.expect(TokenKind::RBrace)?;
                        return Ok(ProofBody::ByMethod(ProofMethod::Contradiction {
                            assumption: Ident::new(Text::from("_contrapositive"), self.stream.current_span()),
                            proof: steps.into_iter().collect(),
                        }));
                    }

                    let tactic = self.parse_tactic_expr_with_commas()?;
                    return Ok(ProofBody::Tactic(tactic));
                }
                _ => {
                    let tactic = self.parse_tactic_expr_with_commas()?;
                    return Ok(ProofBody::Tactic(tactic));
                }
            }
        }

        // proof = term
        if self.stream.consume(&TokenKind::Eq).is_some() {
            let term = self.parse_expr()?;
            return Ok(ProofBody::Term(Heap::new(term)));
        }

        // E025: Check for proof without body (EOF or invalid token after 'proof')
        if self.stream.at_end() {
            return Err(ParseError::invalid_proof_keyword(
                "'proof' keyword requires a body (by tactic, = term, or { ... })",
                self.stream.current_span(),
            ));
        }

        // proof { ... }
        if !self.stream.check(&TokenKind::LBrace) {
            return Err(ParseError::invalid_proof_keyword(
                "'proof' keyword requires a body (by tactic, = term, or { ... })",
                self.stream.current_span(),
            ));
        }

        self.stream.advance(); // consume {
        self.parse_structured_proof_body()
    }

    /// Parse a single proof step.
    fn parse_proof_step(&mut self) -> ParseResult<ProofStep> {
        let start_pos = self.stream.position();

        // Handle `-` bullet point prefix (Lean-style goal bullets)
        // Skip it and parse the following step normally
        if self.stream.check(&TokenKind::Minus) {
            self.stream.advance(); // consume '-'
        }

        let kind = match self.stream.peek_kind() {
            // have name: proposition by justification
            // have: proposition by justification (anonymous)
            Some(TokenKind::Have) => {
                self.stream.advance();
                // Check if next is `:` (anonymous have) or identifier (named have)
                let name = if self.stream.check(&TokenKind::Colon) {
                    // Anonymous have: use generated name
                    Text::from("_anon")
                } else {
                    self.consume_ident()?
                };
                self.stream.expect(TokenKind::Colon)?;
                let proposition = self.parse_expr()?;
                let justification = if self.stream.consume(&TokenKind::By).is_some() {
                    self.parse_tactic_expr_with_commas()?
                } else {
                    TacticExpr::Trivial
                };

                ProofStepKind::Have {
                    name: Ident::new(name, self.stream.current_span()),
                    proposition: Heap::new(proposition),
                    justification,
                }
            }

            // show proposition by justification
            Some(TokenKind::Show) => {
                self.stream.advance();
                let proposition = self.parse_expr()?;
                let justification = if self.stream.consume(&TokenKind::By).is_some() {
                    self.parse_tactic_expr_with_commas()?
                } else {
                    TacticExpr::Trivial
                };

                ProofStepKind::Show {
                    proposition: Heap::new(proposition),
                    justification,
                }
            }

            // suffices to show proposition by justification
            Some(TokenKind::Suffices) => {
                self.stream.advance();
                // Optional "to show"
                if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("to"))) {
                    self.stream.advance();
                    if self.stream.peek_kind() == Some(&TokenKind::Show) {
                        self.stream.advance();
                    }
                }
                let proposition = self.parse_expr()?;
                let justification = if self.stream.consume(&TokenKind::By).is_some() {
                    self.parse_tactic_expr_with_commas()?
                } else {
                    TacticExpr::Trivial
                };

                ProofStepKind::Suffices {
                    proposition: Heap::new(proposition),
                    justification,
                }
            }

            // let pattern = value  OR  let vars such that expr
            Some(TokenKind::Let) => {
                self.stream.advance();
                let pattern = self.parse_pattern()?;

                // Handle comma-separated bindings: let p, q such that expr
                // Skip additional comma-separated identifiers
                while self.stream.consume(&TokenKind::Comma).is_some() {
                    if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("such"))) {
                        break;
                    }
                    // Parse and discard additional binding names
                    let _ = self.parse_pattern()?;
                }

                // Check for "such that" syntax: let a, b such that expr
                if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("such"))) {
                    self.stream.advance(); // consume "such"
                    // expect "that"
                    if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("that"))) {
                        self.stream.advance(); // consume "that"
                    }
                    let constraint = self.parse_expr()?;
                    // Treat as a tactic step (existential elimination)
                    ProofStepKind::Tactic(TacticExpr::Named {
                        name: Ident::new(Text::from("such_that"), self.stream.current_span()),
                        args: List::from_iter(std::iter::once(constraint)),
                        generic_args: List::new(),
                    })
                } else {
                    self.stream.expect(TokenKind::Eq)?;
                    let value = self.parse_expr()?;
                    ProofStepKind::Let {
                        pattern,
                        value: Heap::new(value),
                    }
                }
            }

            // obtain pattern from source
            // Grammar: obtain_step = 'obtain' , pattern , 'from' , expression ;
            Some(TokenKind::Obtain) => {
                self.stream.advance();
                // Parse pattern (struct pattern, tuple pattern, or identifier)
                let pattern = self.parse_pattern()?;
                // "from" keyword
                self.expect_contextual_keyword("from")?;
                let from = self.parse_expr()?;

                ProofStepKind::Obtain {
                    pattern,
                    from: Heap::new(from),
                }
            }

            // calc { ... } - calculation chain
            Some(TokenKind::Calc) => {
                let chain = self.parse_calc_chain()?;
                ProofStepKind::Calc(chain)
            }

            // cases scrutinee { ... } OR cases h (tactic)
            // If followed by braces, parse as structured cases proof step
            // Otherwise, treat as a tactic application (falls through to tactic parsing)
            Some(TokenKind::Cases) => {
                // Look ahead to see if this is a structured cases (with braces)
                let saved_pos = self.stream.position();
                self.stream.advance(); // consume 'cases'

                // Try to parse the expression and check for brace
                // Use parse_expr_no_struct to prevent `b { ... }` from being
                // consumed as a struct literal (the `{` starts the cases block)
                if let Ok(scrutinee) = self.parse_expr_no_struct() {
                    if self.stream.check(&TokenKind::LBrace) {
                        // Check if this is a cases block with case arms or a simple proof block.
                        // Peek after { to see if content looks like case arms (starts with
                        // `case` ident or a pattern followed by =>)
                        let after_brace = self.stream.peek_nth_kind(1);
                        let is_case_arm = matches!(after_brace,
                            Some(TokenKind::Ident(name)) if name.as_str() == "case"
                        ) || matches!(after_brace,
                            Some(TokenKind::True) | Some(TokenKind::False)
                            | Some(TokenKind::Ident(_)) | Some(TokenKind::Integer(_))
                        );
                        let is_proof_step = matches!(after_brace,
                            Some(TokenKind::By) | Some(TokenKind::Proof)
                            | Some(TokenKind::Trivial) | Some(TokenKind::Simp)
                            | Some(TokenKind::Ring) | Some(TokenKind::Omega)
                            | Some(TokenKind::Auto) | Some(TokenKind::Blast)
                            | Some(TokenKind::Smt) | Some(TokenKind::Have)
                            | Some(TokenKind::Show) | Some(TokenKind::Suffices)
                        );

                        if is_proof_step && !is_case_arm {
                            // This is `cases expr { proof_step }` - a short-form case proof
                            self.stream.advance(); // consume '{'
                            let mut steps = Vec::new();
                            while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
                                let step = self.parse_proof_step()?;
                                steps.push(step);
                                self.stream.consume(&TokenKind::Semicolon);
                            }
                            self.stream.expect(TokenKind::RBrace)?;
                            let case_span = self.stream.make_span(start_pos);
                            // Wrap as a single case with wildcard pattern
                            let wildcard = Pattern::new(
                                verum_ast::PatternKind::Wildcard,
                                case_span,
                            );
                            return Ok(ProofStep {
                                kind: ProofStepKind::Cases {
                                    scrutinee: Heap::new(scrutinee),
                                    cases: List::from_iter([ProofCase {
                                        pattern: wildcard,
                                        condition: Maybe::None,
                                        proof: steps.into_iter().collect(),
                                        span: case_span,
                                    }]),
                                },
                                span: case_span,
                            });
                        }

                        self.stream.advance(); // consume '{'
                        let cases = self.parse_proof_cases()?;
                        self.stream.expect(TokenKind::RBrace)?;
                        return Ok(ProofStep {
                            kind: ProofStepKind::Cases {
                                scrutinee: Heap::new(scrutinee),
                                cases: cases.into_iter().collect(),
                            },
                            span: self.stream.make_span(start_pos),
                        });
                    }
                }
                // No brace - restore position and parse as tactic
                self.stream.reset_to(saved_pos);
                let tactic = self.parse_tactic_expr()?;
                ProofStepKind::Tactic(tactic)
            }

            // Proof steps that look like identifiers but need special handling:
            // take x: Type;        -- introduce universally quantified variable
            // witness expr;        -- provide existential witness
            // conclude by tactic;  -- conclude the current goal
            // assume expr;         -- assume a hypothesis
            Some(TokenKind::Ident(name))
                if matches!(
                    name.as_str(),
                    "take" | "witness" | "conclude" | "assume" | "left" | "right"
                        | "Trivial" | "assumed"
                ) =>
            {
                let name_str = name.clone();
                self.stream.advance();
                match name_str.as_str() {
                    "take" => {
                        // take x: Type; -- introduce universally quantified variable
                        let var_name = self.consume_ident()?;
                        let ty = if self.stream.consume(&TokenKind::Colon).is_some() {
                            let t = self.parse_type()?;
                            Maybe::Some(t)
                        } else {
                            Maybe::None
                        };
                        // Consume optional semicolon
                        self.stream.consume(&TokenKind::Semicolon);
                        ProofStepKind::Tactic(TacticExpr::Named {
                            name: Ident::new(Text::from("take"), self.stream.current_span()),
                            generic_args: List::new(),
                            args: List::from_iter(std::iter::once(
                                Expr::ident(Ident::new(var_name, self.stream.current_span())),
                            )),
                        })
                    }
                    "witness" => {
                        // witness expr; -- provide existential witness
                        let expr = self.parse_expr()?;
                        self.stream.consume(&TokenKind::Semicolon);
                        ProofStepKind::Tactic(TacticExpr::Named {
                            name: Ident::new(Text::from("witness"), self.stream.current_span()),
                            args: List::from_iter(std::iter::once(expr)),
                        generic_args: List::new(),
                        })
                    }
                    "conclude" => {
                        // conclude by tactic;  OR  conclude by tactic1, tactic2;
                        let justification = if self.stream.consume(&TokenKind::By).is_some() {
                            self.parse_tactic_expr_with_commas()?
                        } else {
                            TacticExpr::Trivial
                        };
                        ProofStepKind::Tactic(justification)
                    }
                    "assume" => {
                        // assume expr; OR assume name: proposition;
                        // Check for binding pattern: ident ':'
                        let checkpoint = self.stream.position();
                        if self.is_ident() {
                            let ident_name = self.consume_ident()?;
                            if self.stream.consume(&TokenKind::Colon).is_some() {
                                // assume name: proposition; - named hypothesis
                                // Parse as expression (proposition), not type
                                let _prop = self.parse_expr()?;
                                self.stream.consume(&TokenKind::Semicolon);
                                ProofStepKind::Tactic(TacticExpr::Named {
                                    name: Ident::new(Text::from("assume"), self.stream.current_span()),
                                    generic_args: List::new(),
                                    args: List::from_iter(std::iter::once(
                                        Expr::ident(Ident::new(ident_name, self.stream.current_span())),
                                    )),
                                })
                            } else {
                                // Not a binding, restore and parse as expression
                                self.stream.reset_to(checkpoint);
                                let expr = self.parse_expr()?;
                                self.stream.consume(&TokenKind::Semicolon);
                                ProofStepKind::Tactic(TacticExpr::Named {
                                    name: Ident::new(Text::from("assume"), self.stream.current_span()),
                                    args: List::from_iter(std::iter::once(expr)),
                        generic_args: List::new(),
                                })
                            }
                        } else {
                            let expr = self.parse_expr()?;
                            self.stream.consume(&TokenKind::Semicolon);
                            ProofStepKind::Tactic(TacticExpr::Named {
                                name: Ident::new(Text::from("assume"), self.stream.current_span()),
                                args: List::from_iter(std::iter::once(expr)),
                        generic_args: List::new(),
                            })
                        }
                    }
                    "Trivial" | "assumed" => {
                        // Capitalized Trivial, assumed -- treat as trivial tactic
                        ProofStepKind::Tactic(TacticExpr::Trivial)
                    }
                    "left" | "right" => {
                        // left/right -- disjunction tactics
                        self.stream.consume(&TokenKind::Semicolon);
                        ProofStepKind::Tactic(TacticExpr::Named {
                            name: Ident::new(name_str, self.stream.current_span()),
                            args: List::new(),
                        generic_args: List::new(),
                        })
                    }
                    _ => unreachable!(),
                }
            }

            // If-else proof step: if cond { steps } else { steps }
            // Treated as a tactic step containing an expression
            Some(TokenKind::If) => {
                // Parse the entire if expression
                let expr = self.parse_expr()?;
                ProofStepKind::Tactic(TacticExpr::Named {
                    name: Ident::new(Text::from("if_then_else"), self.stream.current_span()),
                    args: List::from_iter(std::iter::once(expr)),
                        generic_args: List::new(),
                })
            }

            // Proof keyword as a proof step (e.g., `proof by omega` inside a block)
            Some(TokenKind::Proof) => {
                let body = self.parse_proof_body()?;
                match body {
                    ProofBody::Tactic(t) => ProofStepKind::Tactic(t),
                    ProofBody::ByMethod(m) => ProofStepKind::Tactic(TacticExpr::Named {
                        name: Ident::new(Text::from("proof_method"), self.stream.current_span()),
                        args: List::new(),
                        generic_args: List::new(),
                    }),
                    _ => ProofStepKind::Tactic(TacticExpr::Trivial),
                }
            }

            // By keyword as a proof step (e.g., `by omega` inside a structured block)
            // Also handles: `by induction on n { case ... }`, `by cases { ... }`
            Some(TokenKind::By) => {
                self.stream.advance(); // consume 'by'
                // Check if this is a proof method (induction/cases/contradiction) with a body
                match self.stream.peek_kind() {
                    Some(TokenKind::Induction) | Some(TokenKind::Cases) | Some(TokenKind::Contradiction) => {
                        let method = self.parse_proof_method_inner()?;
                        ProofStepKind::Tactic(TacticExpr::Named {
                            name: Ident::new(Text::from("_proof_method"), self.stream.current_span()),
                            args: List::new(),
                        generic_args: List::new(),
                        })
                    }
                    // Named proof methods: strong_induction, well_founded_induction, etc.
                    Some(TokenKind::Ident(name)) if {
                        let next = self.stream.peek_nth_kind(1);
                        matches!(next, Some(TokenKind::Ident(n)) if n.as_str() == "on")
                            || matches!(next, Some(TokenKind::LBrace))
                    } => {
                        // Check if this looks like a proof method with cases
                        let checkpoint = self.stream.position();
                        let _method_name = self.consume_ident()?;
                        if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("on"))) {
                            self.stream.advance(); // consume "on"
                            // Skip the "on" target
                            if self.stream.check(&TokenKind::LParen) {
                                let _ = self.parse_expr()?;
                            } else if self.is_ident() {
                                self.stream.advance();
                            }
                            // Skip comma-separated additional targets
                            while self.stream.consume(&TokenKind::Comma).is_some() {
                                if self.stream.check(&TokenKind::LBrace) || self.stream.at_end() { break; }
                                if self.is_ident() { self.stream.advance(); }
                            }
                        }
                        // Parse optional cases block
                        if self.stream.check(&TokenKind::LBrace) {
                            self.stream.advance();
                            let cases = self.parse_proof_cases()?;
                            self.stream.expect(TokenKind::RBrace)?;
                        }
                        ProofStepKind::Tactic(TacticExpr::Named {
                            name: Ident::new(Text::from("_proof_method"), self.stream.current_span()),
                            args: List::new(),
                        generic_args: List::new(),
                        })
                    }
                    _ => {
                        let tactic = self.parse_tactic_expr_with_commas()?;
                        ProofStepKind::Tactic(tactic)
                    }
                }
            }

            // Tactic application: tactic_expr ';'
            // Grammar: tactic_application = tactic_expr , ';' ;
            // This handles bare tactics like: trivial; simp; ring; auto; cases h;
            Some(TokenKind::Trivial)
            | Some(TokenKind::Assumption)
            | Some(TokenKind::Contradiction)
            | Some(TokenKind::Simp)
            | Some(TokenKind::Ring)
            | Some(TokenKind::Field)
            | Some(TokenKind::Omega)
            | Some(TokenKind::Auto)
            | Some(TokenKind::Blast)
            | Some(TokenKind::Smt)
            | Some(TokenKind::Induction)
            | Some(TokenKind::Ident(_))
            | Some(TokenKind::LParen)
            | Some(TokenKind::LBrace) => {
                let tactic = self.parse_tactic_expr()?;
                // Tactic applications don't need the semicolon consumed here
                // because the caller (parse_structured_proof_body) already handles optional semicolons
                ProofStepKind::Tactic(tactic)
            }

            // Fallback: try to parse as an expression-based proof step
            // This handles proof expressions, function calls, method calls, etc.
            _ => {
                if let Some(expr) = self.optional(|p| p.parse_expr()) {
                    self.stream.consume(&TokenKind::Semicolon);
                    ProofStepKind::Tactic(TacticExpr::Named {
                        name: Ident::new(Text::from("_expr"), self.stream.current_span()),
                        args: List::from_iter(std::iter::once(expr)),
                        generic_args: List::new(),
                    })
                } else {
                    return Err(ParseError::invalid_syntax(
                        "expected proof step (have, show, suffices, let, obtain, calc, cases, or tactic)",
                        self.stream.current_span(),
                    ));
                }
            }
        };

        let span = self.stream.make_span(start_pos);
        Ok(ProofStep { kind, span })
    }

    /// Parse witnesses as identifiers: (x, y, z)
    fn parse_witness_idents(&mut self) -> ParseResult<List<Ident>> {
        if self.stream.check(&TokenKind::LParen) {
            self.stream.advance();
            let witnesses = self.comma_separated(|p| {
                let name = p.consume_ident()?;
                Ok(Ident::new(name, p.stream.current_span()))
            })?;
            self.stream.expect(TokenKind::RParen)?;
            Ok(witnesses.into_iter().collect())
        } else {
            // Single witness
            let name = self.consume_ident()?;
            Ok(List::from_iter([Ident::new(
                name,
                self.stream.current_span(),
            )]))
        }
    }

    /// Parse proof cases: case pattern => { steps }
    fn parse_proof_cases(&mut self) -> ParseResult<Vec<ProofCase>> {
        let mut cases = Vec::new();

        while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
            let start_pos = self.stream.position();

            // "case" keyword (contextual) - optional to support both:
            // `case true => { ... }` and `true => { ... }`
            if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("case"))) {
                self.stream.advance();
            }

            // Case pattern or expression predicate. `condition`
            // collects the Expr for guard-shape cases (`case a >= b`)
            // so the verifier can thread it as a hypothesis; it
            // stays `None` for real constructor / literal patterns.
            let mut condition: Maybe<Heap<Expr>> = Maybe::None;
            // Try pattern first. If after pattern we don't see => or { or where,
            // backtrack and parse as expression (for `case x >= 0 =>` style)
            // Use parse_expr_bp(4) to stop before FatArrow (=>), which has bp=3.
            // This prevents `x == 0 => trivial` from being consumed as an implication.
            let checkpoint = self.stream.position();
            let pattern = if let Some(pat) = self.optional(|p| p.parse_pattern()) {
                // Check if what follows looks like case continuation
                if self.stream.check(&TokenKind::FatArrow)
                    || self.stream.check(&TokenKind::LBrace)
                    || self.stream.check(&TokenKind::Where)
                    || self.stream.check(&TokenKind::Comma)
                    || self.stream.check(&TokenKind::RBrace)
                {
                    pat
                } else if self.is_ident() {
                    // Constructor pattern with space-separated argument: `left a`, `succ(k)`
                    // Consume additional argument identifiers until we see => or {
                    while self.is_ident()
                        && !self.stream.check(&TokenKind::FatArrow)
                        && !self.stream.check(&TokenKind::LBrace)
                        && !self.stream.check(&TokenKind::Where)
                    {
                        self.stream.advance();
                        // If we see `(`, consume a parenthesized sub-expression
                        if self.stream.check(&TokenKind::LParen) {
                            let _ = self.parse_expr()?;
                        }
                    }
                    pat
                } else {
                    // Not a valid case continuation - parse as expression instead
                    self.stream.reset_to(checkpoint);
                    let expr = self.parse_expr_bp(4)?;
                    let pat = Pattern::new(
                        verum_ast::PatternKind::Literal(verum_ast::Literal::bool(true, expr.span)),
                        expr.span,
                    );
                    // Preserve the expression as the case condition so
                    // the verifier can thread it as a hypothesis into
                    // the case body.
                    condition = Maybe::Some(Heap::new(expr));
                    pat
                }
            } else {
                // Parse as expression and wrap in a pattern
                let expr = self.parse_expr_bp(4)?;
                let pat = Pattern::new(
                    verum_ast::PatternKind::Literal(verum_ast::Literal::bool(true, expr.span)),
                    expr.span,
                );
                condition = Maybe::Some(Heap::new(expr));
                pat
            };

            // Optional where guard: case n where cond => ...
            // Parse and capture the guard expression so the verifier
            // can use it alongside (or in place of) the case condition
            // as a hypothesis in the body.
            if self.stream.consume(&TokenKind::Where).is_some() {
                let guard = self.parse_expr_no_struct()?;
                // If no fallback-expression condition was set, use the
                // where-guard as the case condition. If both are set,
                // prefer the `where` guard — it's semantically the
                // stronger surface form.
                condition = Maybe::Some(Heap::new(guard));
            }

            // => or {
            if self.stream.consume(&TokenKind::FatArrow).is_some() {
                // Single expression/tactic
                if self.stream.check(&TokenKind::LBrace) {
                    let steps = self.parse_proof_case_body()?;
                    cases.push(ProofCase {
                        pattern,
                        condition: condition.clone(),
                        proof: steps.into_iter().collect(),
                        span: self.stream.make_span(start_pos),
                    });
                } else if self.stream.check(&TokenKind::Calc) {
                    // Calc chain as the case proof
                    let chain = self.parse_calc_chain()?;
                    let calc_span = chain.span;
                    cases.push(ProofCase {
                        pattern,
                        condition: condition.clone(),
                        proof: List::from_iter([ProofStep {
                            kind: ProofStepKind::Calc(chain),
                            span: calc_span,
                        }]),
                        span: self.stream.make_span(start_pos),
                    });
                } else {
                    let tactic = self.parse_tactic_expr()?;
                    let case_span = self.stream.make_span(start_pos);
                    // When a proof case has a single tactic without a body,
                    // we synthesize a "show true" proposition. Use the case_span
                    // for the synthetic literal to ensure proper error reporting.
                    cases.push(ProofCase {
                        pattern,
                        condition: condition.clone(),
                        proof: List::from_iter([ProofStep {
                            kind: ProofStepKind::Show {
                                proposition: Heap::new(Expr::new(
                                    ExprKind::Literal(verum_ast::Literal::bool(true, case_span)),
                                    case_span,
                                )),
                                justification: tactic,
                            },
                            span: case_span,
                        }]),
                        span: case_span,
                    });
                }
            } else {
                self.stream.expect(TokenKind::LBrace)?;
                let steps = self.parse_proof_case_body()?;
                self.stream.expect(TokenKind::RBrace)?;
                cases.push(ProofCase {
                    pattern,
                    condition: condition.clone(),
                    proof: steps.into_iter().collect(),
                    span: self.stream.make_span(start_pos),
                });
            }

            // Optional comma between cases
            self.stream.consume(&TokenKind::Comma);
        }

        Ok(cases)
    }

    /// Parse proof case body.
    fn parse_proof_case_body(&mut self) -> ParseResult<Vec<ProofStep>> {
        self.stream.expect(TokenKind::LBrace)?;

        let mut steps = Vec::new();
        while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
            let step = self.parse_proof_step()?;
            steps.push(step);
            self.stream.consume(&TokenKind::Semicolon);
        }

        self.stream.expect(TokenKind::RBrace)?;
        Ok(steps)
    }

    /// Parse proof by method.
    fn parse_proof_by_method(&mut self) -> ParseResult<ProofBody> {
        self.stream.expect(TokenKind::By)?;
        let method = self.parse_proof_method_inner()?;
        Ok(ProofBody::ByMethod(method))
    }

    /// Parse the inner proof method after 'by'.
    fn parse_proof_method_inner(&mut self) -> ParseResult<ProofMethod> {
        match self.stream.peek_kind() {
            Some(TokenKind::Induction) => {
                self.stream.advance();

                // Optional "on" variable: `induction on n` or just `induction n`
                let on = if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("on"))) {
                    self.stream.advance(); // consume "on"
                    Maybe::Some(Ident::new(
                        self.consume_ident()?,
                        self.stream.current_span(),
                    ))
                } else if self.stream.check(&TokenKind::LParen) {
                    // Support `induction(n)` with parenthesized argument
                    self.stream.advance(); // consume '('
                    let name = self.consume_ident()?;
                    let span = self.stream.current_span();
                    self.stream.expect(TokenKind::RParen)?;
                    Maybe::Some(Ident::new(name, span))
                } else if self.is_ident() && !self.stream.check(&TokenKind::LBrace) {
                    // Support `induction n` without explicit "on" keyword
                    Maybe::Some(Ident::new(
                        self.consume_ident()?,
                        self.stream.current_span(),
                    ))
                } else {
                    Maybe::None
                };

                // Optional cases block - if no brace, treat as auto-induction
                if self.stream.check(&TokenKind::LBrace) {
                    self.stream.advance();
                    let cases = self.parse_proof_cases()?;
                    self.stream.expect(TokenKind::RBrace)?;
                    Ok(ProofMethod::Induction {
                        on,
                        cases: cases.into_iter().collect(),
                    })
                } else {
                    // No cases block - auto induction
                    Ok(ProofMethod::Induction {
                        on,
                        cases: verum_common::List::new(),
                    })
                }
            }

            Some(TokenKind::Cases) => {
                self.stream.advance();

                // Optional "on" keyword for scrutinee: `cases on x { ... }` or `cases { ... }`
                // Also handle bare `cases;` (no scrutinee, no block).
                let on = if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("on"))) {
                    self.stream.advance(); // consume "on"
                    self.parse_expr_no_struct()?
                } else if self.stream.check(&TokenKind::LBrace) {
                    // No scrutinee - empty expression placeholder
                    Expr::new(
                        ExprKind::Tuple(List::new()),
                        self.stream.current_span(),
                    )
                } else if self.stream.check(&TokenKind::Semicolon) || self.stream.at_end()
                    || self.stream.check(&TokenKind::RBrace)
                {
                    // Bare `cases;` or `cases` at end - no scrutinee
                    Expr::new(
                        ExprKind::Tuple(List::new()),
                        self.stream.current_span(),
                    )
                } else {
                    self.parse_expr_no_struct()?
                };

                // Optional cases block - if no brace, treat as auto case analysis
                if self.stream.check(&TokenKind::LBrace) {
                    self.stream.advance();
                    let cases = self.parse_proof_cases()?;
                    self.stream.expect(TokenKind::RBrace)?;
                    Ok(ProofMethod::Cases {
                        on: Heap::new(on),
                        cases: cases.into_iter().collect(),
                    })
                } else {
                    Ok(ProofMethod::Cases {
                        on: Heap::new(on),
                        cases: verum_common::List::new(),
                    })
                }
            }

            Some(TokenKind::Contradiction) => {
                self.stream.advance();

                // Optional assumption name
                let assumption = if self.is_ident() {
                    Ident::new(self.consume_ident()?, self.stream.current_span())
                } else {
                    Ident::new(Text::from("_contradiction"), self.stream.current_span())
                };

                // Optional proof block
                if self.stream.check(&TokenKind::LBrace) {
                    self.stream.advance();
                    let mut proof = Vec::new();
                    while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
                        let step = self.parse_proof_step()?;
                        proof.push(step);
                        self.stream.consume(&TokenKind::Semicolon);
                    }
                    self.stream.expect(TokenKind::RBrace)?;
                    Ok(ProofMethod::Contradiction {
                        assumption,
                        proof: proof.into_iter().collect(),
                    })
                } else {
                    Ok(ProofMethod::Contradiction {
                        assumption,
                        proof: verum_common::List::new(),
                    })
                }
            }

            _ => Err(ParseError::invalid_syntax(
                "expected proof method (induction, cases, contradiction)",
                self.stream.current_span(),
            )),
        }
    }

    // ============================================================================
    // Calculation Chains
    // ============================================================================

    /// Parse a calculation chain.
    /// Syntax: calc { start = expr by justification; = expr by justification; ... }
    ///
    /// Calculation chains enable equational reasoning:
    /// `calc { start = expr by justification; = expr by justification; ... }`
    /// Relations: = (equality), != (inequality), <, <=, >, >=, => (implication), iff.
    pub fn parse_calc_chain(&mut self) -> ParseResult<CalculationChain> {
        let start_pos = self.stream.position();

        self.stream.expect(TokenKind::Calc)?;
        self.stream.expect(TokenKind::LBrace)?;

        // Parse starting expression - use binding power 7 to stop at comparison operators
        // This allows `a + b` to be parsed as the start, with `==` being the calc relation
        let start = self.parse_expr_bp(7)?;

        // Parse steps
        let mut steps = Vec::new();
        while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
            let step_start = self.stream.position();

            // Parse relation (=, <, <=, etc.)
            let relation = self.parse_calc_relation()?;

            // Calc chain supports two syntax forms:
            // 1. == { by justification } target_expr
            // 2. == target_expr by justification
            //
            // Target expressions use binding power 7 to stop at comparison operators,
            // allowing the next step's relation to be recognized correctly.
            let (justification, target) = if self.stream.check(&TokenKind::LBrace) {
                // Syntax: == { by justification } target_expr
                self.stream.advance(); // consume {
                let just = if self.stream.consume(&TokenKind::By).is_some() {
                    self.parse_tactic_expr_with_commas()?
                } else {
                    TacticExpr::Trivial
                };
                self.stream.expect(TokenKind::RBrace)?;
                let tgt = self.parse_expr_bp(7)?;
                (just, tgt)
            } else {
                // Syntax: == target_expr by justification
                let tgt = self.parse_expr_bp(7)?;
                let just = if self.stream.consume(&TokenKind::By).is_some() {
                    self.parse_tactic_expr_with_commas()?
                } else {
                    TacticExpr::Trivial
                };
                (just, tgt)
            };

            let step_span = self.stream.make_span(step_start);
            steps.push(CalculationStep {
                relation,
                target: Heap::new(target),
                justification,
                span: step_span,
            });

            // Optional semicolon
            self.stream.consume(&TokenKind::Semicolon);
        }

        self.stream.expect(TokenKind::RBrace)?;

        let span = self.stream.make_span(start_pos);
        Ok(CalculationChain {
            start: Heap::new(start),
            steps: steps.into_iter().collect(),
            span,
        })
    }

    /// Parse a calculation relation.
    fn parse_calc_relation(&mut self) -> ParseResult<CalcRelation> {
        match self.stream.peek_kind() {
            Some(TokenKind::EqEq) | Some(TokenKind::Eq) => {
                self.stream.advance();
                Ok(CalcRelation::Eq)
            }
            Some(TokenKind::BangEq) => {
                self.stream.advance();
                Ok(CalcRelation::Ne)
            }
            Some(TokenKind::Lt) => {
                self.stream.advance();
                Ok(CalcRelation::Lt)
            }
            Some(TokenKind::LtEq) => {
                self.stream.advance();
                Ok(CalcRelation::Le)
            }
            Some(TokenKind::Gt) => {
                self.stream.advance();
                Ok(CalcRelation::Gt)
            }
            Some(TokenKind::GtEq) => {
                self.stream.advance();
                Ok(CalcRelation::Ge)
            }
            // Check for implies (=>)
            Some(TokenKind::FatArrow) => {
                self.stream.advance();
                Ok(CalcRelation::Implies)
            }
            // Check for iff (<=>)
            Some(TokenKind::Ident(name)) if name.as_str() == "iff" => {
                self.stream.advance();
                Ok(CalcRelation::Iff)
            }
            _ => Err(ParseError::invalid_syntax(
                "expected calculation relation (=, !=, <, <=, >, >=, =>, iff)",
                self.stream.current_span(),
            )),
        }
    }

    // ============================================================================
    // Tactic Expressions
    // ============================================================================

    /// Parse a tactic expression.
    ///
    /// Tactic expressions: simp, ring, field, omega, blast, trivial, assumption,
    /// exact(term), apply(term), rewrite[lemma], unfold(def), intro(names),
    /// specialize(term, args), norm_num, decide, ext, funext, congr, and more.
    pub fn parse_tactic_expr(&mut self) -> ParseResult<TacticExpr> {
        let tactic = self.parse_tactic_or()?;

        // Handle optional `using` modifier: `tactic using [hints]` or `tactic using hint`
        // This allows: `proof by smt using LRA`, `proof by auto using [lemma1, lemma2]`
        if self.stream.check(&TokenKind::Using) {
            self.stream.advance();
            // Parse the hints - either a bracketed list or a single identifier
            let _hints = if self.stream.check(&TokenKind::LBracket) {
                self.stream.advance();
                let hints = self.comma_separated(|p| p.parse_expr())?;
                self.stream.expect(TokenKind::RBracket)?;
                hints
            } else if self.is_ident() {
                // Single hint or hint category followed by [hint_list]
                // Parse just the identifier (not as a full expression) to prevent
                // `using hints [list]` from being parsed as `hints[list]` index
                let hint_name = self.consume_ident()?;
                let hint = Expr::ident(Ident::new(hint_name, self.stream.current_span()));
                // Check for optional bracket list after hint category name
                // E.g., `using hints [list_hints, set_hints]`
                if self.stream.check(&TokenKind::LBracket) {
                    self.stream.advance();
                    let mut hints = self.comma_separated(|p| p.parse_expr())?;
                    self.stream.expect(TokenKind::RBracket)?;
                    hints.insert(0, hint);
                    hints
                } else {
                    vec![hint]
                }
            } else {
                // Fallback: parse as expression
                let hint = self.parse_expr()?;
                vec![hint]
            };
            // For now, we discard the hints and return the base tactic
            // A future implementation would attach hints to the tactic
            Ok(tactic)
        } else {
            Ok(tactic)
        }
    }

    /// Parse a tactic expression that may include comma-separated justifications.
    /// Handles patterns like: `by ih1, ih2, arithmetic` or `by definition, arithmetic`
    /// Commas create a sequence of tactics (equivalent to `;` separation).
    /// This is used specifically in proof justification contexts (after `by`).
    pub fn parse_tactic_expr_with_commas(&mut self) -> ParseResult<TacticExpr> {
        let first = self.parse_tactic_expr()?;

        // Check for comma-separated continuation
        if !self.stream.check(&TokenKind::Comma) {
            return Ok(first);
        }

        let mut tactics = vec![first];
        while self.stream.consume(&TokenKind::Comma).is_some() {
            // Stop at delimiters that indicate end of tactic list
            if self.stream.check(&TokenKind::RBrace)
                || self.stream.check(&TokenKind::Semicolon)
                || self.stream.check(&TokenKind::RBracket)
                || self.stream.check(&TokenKind::RParen)
                || self.stream.at_end()
            {
                break;
            }
            tactics.push(self.parse_tactic_expr()?);
        }

        if tactics.len() == 1 {
            Ok(tactics.pop().expect("tactics.len() == 1"))
        } else {
            Ok(TacticExpr::Seq(tactics.into_iter().collect()))
        }
    }

    /// Parse tactic alternatives: t1 | t2
    fn parse_tactic_or(&mut self) -> ParseResult<TacticExpr> {
        let mut tactic = self.parse_tactic_seq()?;

        while self.stream.consume(&TokenKind::Pipe).is_some() {
            let right = self.parse_tactic_seq()?;
            tactic = TacticExpr::Alt(List::from_iter([tactic, right]));
        }

        Ok(tactic)
    }

    /// Parse tactic sequence: t1; t2
    fn parse_tactic_seq(&mut self) -> ParseResult<TacticExpr> {
        let mut tactics = vec![self.parse_tactic_primary()?];

        while self.stream.check(&TokenKind::Semicolon) {
            // Check if what follows the semicolon is a proof step keyword - if so, don't consume it
            // as the semicolon belongs to the proof step, not the tactic sequence
            let next_after_semi = self.stream.peek_nth_kind(1);
            let is_proof_step_or_item = matches!(
                next_after_semi,
                Some(TokenKind::Have)
                    | Some(TokenKind::Show)
                    | Some(TokenKind::Suffices)
                    | Some(TokenKind::Let)
                    | Some(TokenKind::Obtain)
                    | Some(TokenKind::Calc)
                    | Some(TokenKind::Cases)
                    | Some(TokenKind::RBrace)
                    | Some(TokenKind::Qed)
                    // Top-level item keywords — stop tactic sequence at item boundaries
                    | Some(TokenKind::Fn)
                    | Some(TokenKind::Type)
                    | Some(TokenKind::Implement)
                    | Some(TokenKind::Theorem)
                    | Some(TokenKind::Lemma)
                    | Some(TokenKind::Corollary)
                    | Some(TokenKind::Axiom)
                    | Some(TokenKind::Module)
                    | Some(TokenKind::Mount)
                    | Some(TokenKind::Const)
                    | Some(TokenKind::Static)
                    | Some(TokenKind::Pub)
                    | Some(TokenKind::Public)
                    | Some(TokenKind::Internal)
                    | Some(TokenKind::Protected)
                    | Some(TokenKind::At)
                    // End-of-file: the lexer emits an explicit Eof token
                    // *and* `peek_nth_kind` may also return `None` when we
                    // ask past the last token. Handle both.
                    | Some(TokenKind::Eof)
                    | None
            );

            // Also check for contextual proof step keywords (identifiers used as proof commands)
            let is_proof_step_or_item = is_proof_step_or_item || matches!(
                next_after_semi,
                Some(TokenKind::Ident(name)) if matches!(
                    name.as_str(),
                    "conclude" | "assume" | "take" | "witness" | "left" | "right"
                    | "Trivial" | "assumed"
                )
            );

            if is_proof_step_or_item {
                break;
            }

            // Safe to consume semicolon and continue tactic sequence
            self.stream.advance();
            tactics.push(self.parse_tactic_primary()?);
        }

        if tactics.len() == 1 {
            // SAFETY: len() == 1 guarantees pop() returns Some
            Ok(tactics.pop().expect("tactics.len() == 1 guarantees one element"))
        } else {
            Ok(TacticExpr::Seq(tactics.into_iter().collect()))
        }
    }

    /// Parse primary tactic expressions.
    pub(crate) fn parse_tactic_primary(&mut self) -> ParseResult<TacticExpr> {
        match self.stream.peek_kind() {
            // Let-binding inside a tactic body:
            //   `let x: T = expr;` or `let x = expr;`
            // The semicolon after the expression is consumed by the caller
            // (`parse_tactic_seq`/`parse_tactic_body`), which treats `;` as
            // the tactic-sequence separator.
            Some(TokenKind::Let) => {
                self.stream.advance();
                let name_text = self.consume_ident()?;
                let name_span = self.stream.current_span();
                let ty = if self.stream.consume(&TokenKind::Colon).is_some() {
                    Maybe::Some(self.parse_type()?)
                } else {
                    Maybe::None
                };
                self.stream.expect(TokenKind::Eq)?;
                let value = self.parse_expr()?;
                Ok(TacticExpr::Let {
                    name: Ident::new(name_text, name_span),
                    ty,
                    value: Heap::new(value),
                })
            }

            // Conditional tactic execution:
            //   `if cond { t1 } else { t2 }`
            // Both branches are tactic expressions; the else branch is
            // optional. Desugars `else if` chains through nested `If`.
            // Use `parse_expr_no_struct` so `if x { ... }` doesn't get
            // eaten as a struct literal `x { ... }`.
            Some(TokenKind::If) => {
                self.stream.advance();
                let cond = self.parse_expr_no_struct()?;
                self.stream.expect(TokenKind::LBrace)?;
                let then_branch = self.parse_tactic_expr()?;
                // Consume optional trailing `;` inside the block.
                self.stream.consume(&TokenKind::Semicolon);
                self.stream.expect(TokenKind::RBrace)?;
                let else_branch = if self.stream.consume(&TokenKind::Else).is_some() {
                    // Accept either `else { body }` or `else if ...`
                    if self.stream.check(&TokenKind::If) {
                        Maybe::Some(Heap::new(self.parse_tactic_primary()?))
                    } else {
                        self.stream.expect(TokenKind::LBrace)?;
                        let body = self.parse_tactic_expr()?;
                        self.stream.consume(&TokenKind::Semicolon);
                        self.stream.expect(TokenKind::RBrace)?;
                        Maybe::Some(Heap::new(body))
                    }
                } else {
                    Maybe::None
                };
                Ok(TacticExpr::If {
                    cond: Heap::new(cond),
                    then_branch: Heap::new(then_branch),
                    else_branch,
                })
            }

            // Pattern-match inside a tactic body:
            //   `match scrutinee { P1 => t1, P2 => t2, ... }`
            Some(TokenKind::Match) if self.stream.peek_nth_kind(1) != Some(&TokenKind::LParen) => {
                self.stream.advance();
                // Use `parse_expr_no_struct` so that `match x {` does not
                // get greedily parsed as a record-literal `x { ... }`.
                let scrutinee = self.parse_expr_no_struct()?;
                self.stream.expect(TokenKind::LBrace)?;
                let mut arms = Vec::new();
                while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
                    let arm_start = self.stream.position();
                    let pattern = self.parse_pattern()?;
                    let guard = if self.stream.consume(&TokenKind::If).is_some() {
                        Maybe::Some(Heap::new(self.parse_expr()?))
                    } else {
                        Maybe::None
                    };
                    self.stream.expect(TokenKind::FatArrow)?;
                    let body = self.parse_tactic_expr()?;
                    let span = self.stream.make_span(arm_start);
                    arms.push(verum_ast::decl::TacticMatchArm {
                        pattern,
                        guard,
                        body: Heap::new(body),
                        span,
                    });
                    // Separator between arms — `,` or `;`, both optional at end.
                    if self.stream.consume(&TokenKind::Comma).is_none() {
                        self.stream.consume(&TokenKind::Semicolon);
                    }
                }
                self.stream.expect(TokenKind::RBrace)?;
                Ok(TacticExpr::Match {
                    scrutinee: Heap::new(scrutinee),
                    arms: arms.into_iter().collect(),
                })
            }

            // Explicit failure with a diagnostic message:
            //   `fail("reason")` or `fail(f"tmpl {x}")`
            Some(TokenKind::Ident(name)) if name.as_str() == "fail" => {
                self.stream.advance();
                // Accept `fail(msg)` or `fail msg`.
                let message = if self.stream.consume(&TokenKind::LParen).is_some() {
                    let msg = self.parse_expr()?;
                    self.stream.expect(TokenKind::RParen)?;
                    msg
                } else {
                    self.parse_expr()?
                };
                Ok(TacticExpr::Fail {
                    message: Heap::new(message),
                })
            }

            // Basic tactics
            Some(TokenKind::Trivial) => {
                self.stream.advance();
                Ok(TacticExpr::Trivial)
            }
            Some(TokenKind::Assumption) => {
                self.stream.advance();
                Ok(TacticExpr::Assumption)
            }
            Some(TokenKind::Ident(name))
                if name.as_str() == "reflexivity" || name.as_str() == "refl" =>
            {
                self.stream.advance();
                Ok(TacticExpr::Reflexivity)
            }

            // Simplification tactics
            Some(TokenKind::Simp) => {
                self.stream.advance();
                // Optional lemmas: [lemma1, lemma2]
                let lemmas = if self.stream.check(&TokenKind::LBracket) {
                    self.stream.advance();
                    let lemmas = self.comma_separated(|p| p.parse_expr())?;
                    self.stream.expect(TokenKind::RBracket)?;
                    lemmas.into_iter().collect()
                } else {
                    List::new()
                };
                // Optional at target
                let at_target =
                    if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("at"))) {
                        self.stream.advance();
                        Maybe::Some(Ident::new(
                            self.consume_ident()?,
                            self.stream.current_span(),
                        ))
                    } else {
                        Maybe::None
                    };
                Ok(TacticExpr::Simp { lemmas, at_target })
            }

            // Arithmetic tactics
            Some(TokenKind::Ring) => {
                self.stream.advance();
                Ok(TacticExpr::Ring)
            }
            Some(TokenKind::Field) => {
                self.stream.advance();
                Ok(TacticExpr::Field)
            }
            Some(TokenKind::Omega) => {
                self.stream.advance();
                // Skip optional config block: omega { theory: divmod }
                if self.stream.check(&TokenKind::LBrace) {
                    self.skip_balanced_braces();
                }
                Ok(TacticExpr::Omega)
            }

            // Automated tactics
            Some(TokenKind::Auto) => {
                self.stream.advance();
                // Optional "with hints"
                let with_hints =
                    if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("with"))) {
                        self.stream.advance();
                        // Parse hints as identifier list
                        let hint_name = self.consume_ident()?;
                        List::from_iter([Ident::new(hint_name, self.stream.current_span())])
                    } else {
                        List::new()
                    };
                // Skip optional config block: auto { depth: 10, ... }
                if self.stream.check(&TokenKind::LBrace) {
                    self.skip_balanced_braces();
                }
                Ok(TacticExpr::Auto { with_hints })
            }
            Some(TokenKind::Blast) => {
                self.stream.advance();
                // Skip optional config block: blast { depth: 5 }
                if self.stream.check(&TokenKind::LBrace) {
                    self.skip_balanced_braces();
                }
                Ok(TacticExpr::Blast)
            }
            Some(TokenKind::Smt) => {
                self.stream.advance();
                // Skip optional config block: smt { solver: z3, timeout: 5000 }
                if self.stream.check(&TokenKind::LBrace) {
                    self.skip_balanced_braces();
                }
                let solver = Maybe::None;
                let timeout = Maybe::None;
                Ok(TacticExpr::Smt { solver, timeout })
            }

            // Contradiction tactic
            Some(TokenKind::Contradiction) => {
                self.stream.advance();
                Ok(TacticExpr::Contradiction)
            }

            // Intro tactic
            Some(TokenKind::Ident(name)) if name.as_str() == "intro" => {
                self.stream.advance();
                let names = if self.stream.check(&TokenKind::LParen) {
                    self.stream.advance();
                    let names: Vec<Ident> = self.comma_separated(|p| {
                        let name = p.consume_ident()?;
                        Ok(Ident::new(name, p.stream.current_span()))
                    })?;
                    self.stream.expect(TokenKind::RParen)?;
                    names.into_iter().collect()
                } else if let Some(TokenKind::Ident(_)) = self.stream.peek_kind() {
                    let name = self.consume_ident()?;
                    List::from_iter([Ident::new(name, self.stream.current_span())])
                } else {
                    List::new()
                };
                Ok(TacticExpr::Intro(names))
            }

            // Apply tactic
            Some(TokenKind::Ident(name)) if name.as_str() == "apply" => {
                self.stream.advance();
                let lemma = self.parse_expr()?;
                Ok(TacticExpr::Apply {
                    lemma: Heap::new(lemma),
                    args: List::new(),
                })
            }

            // Rewrite tactic
            Some(TokenKind::Ident(name)) if name.as_str() == "rewrite" || name.as_str() == "rw" => {
                self.stream.advance();
                // Optional reverse
                let rev = self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("←")))
                    || self.stream.peek_kind() == Some(&TokenKind::Lt);
                if rev {
                    self.stream.advance();
                }
                // Parse the hypothesis expression (usually a path)
                let hypothesis_expr = self.parse_expr()?;
                // Optional at target
                let at_target =
                    if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("at"))) {
                        self.stream.advance();
                        Maybe::Some(Ident::new(
                            self.consume_ident()?,
                            self.stream.current_span(),
                        ))
                    } else {
                        Maybe::None
                    };
                Ok(TacticExpr::Rewrite {
                    hypothesis: Heap::new(hypothesis_expr),
                    at_target,
                    rev,
                })
            }

            // Split tactic
            Some(TokenKind::Ident(name)) if name.as_str() == "split" => {
                self.stream.advance();
                Ok(TacticExpr::Split)
            }

            // Left/Right tactics
            Some(TokenKind::Ident(name)) if name.as_str() == "left" => {
                self.stream.advance();
                Ok(TacticExpr::Left)
            }
            Some(TokenKind::Ident(name)) if name.as_str() == "right" => {
                self.stream.advance();
                Ok(TacticExpr::Right)
            }

            // Exists tactic
            Some(TokenKind::Ident(name)) if name.as_str() == "exists" || name.as_str() == "use" => {
                self.stream.advance();
                let witness = self.parse_expr()?;
                Ok(TacticExpr::Exists(Heap::new(witness)))
            }

            // Induction tactic: induction n, induction(n), or induction on n
            Some(TokenKind::Induction) => {
                self.stream.advance();
                // Optional "on" keyword
                if self.stream.peek_kind() == Some(&TokenKind::Ident(Text::from("on"))) {
                    self.stream.advance();
                }
                // Accept either `induction n` or `induction(n)`
                let var = if self.stream.check(&TokenKind::LParen) {
                    self.stream.advance();
                    let name = self.consume_ident()?;
                    let var = Ident::new(name, self.stream.current_span());
                    self.stream.expect(TokenKind::RParen)?;
                    var
                } else {
                    Ident::new(self.consume_ident()?, self.stream.current_span())
                };
                Ok(TacticExpr::InductionOn(var))
            }

            // Cases tactic: cases, cases n, or cases(n)
            Some(TokenKind::Cases) => {
                self.stream.advance();
                // Accept `cases`, `cases n`, or `cases(n)` — argument is optional
                let var = if self.stream.check(&TokenKind::LParen) {
                    self.stream.advance();
                    let name = self.consume_ident()?;
                    let var = Ident::new(name, self.stream.current_span());
                    self.stream.expect(TokenKind::RParen)?;
                    Maybe::Some(var)
                } else if self.stream.peek_kind().is_some_and(|k| matches!(k, TokenKind::Ident(_))) {
                    Maybe::Some(Ident::new(self.consume_ident()?, self.stream.current_span()))
                } else {
                    Maybe::None
                };
                match var {
                    Maybe::Some(v) => Ok(TacticExpr::CasesOn(v)),
                    Maybe::None => Ok(TacticExpr::Auto { with_hints: List::new() }), // cases without arg treated as auto case analysis
                }
            }

            // Exact tactic
            Some(TokenKind::Ident(name)) if name.as_str() == "exact" => {
                self.stream.advance();
                let proof = self.parse_expr()?;
                Ok(TacticExpr::Exact(Heap::new(proof)))
            }

            // Unfold tactic
            Some(TokenKind::Ident(name)) if name.as_str() == "unfold" => {
                self.stream.advance();
                let names = if self.stream.check(&TokenKind::LBracket) {
                    self.stream.advance();
                    let names: Vec<Ident> = self.comma_separated(|p| {
                        let name = p.consume_ident()?;
                        Ok(Ident::new(name, p.stream.current_span()))
                    })?;
                    self.stream.expect(TokenKind::RBracket)?;
                    names.into_iter().collect()
                } else if self.stream.check(&TokenKind::LParen) {
                    self.stream.advance();
                    let names: Vec<Ident> = self.comma_separated(|p| {
                        let name = p.consume_ident()?;
                        Ok(Ident::new(name, p.stream.current_span()))
                    })?;
                    self.stream.expect(TokenKind::RParen)?;
                    names.into_iter().collect()
                } else if self.is_ident() {
                    // Parse qualified path: List.len, Map.get, etc.
                    let mut full_name = self.consume_ident()?;
                    let start_span = self.stream.current_span();
                    while self.stream.consume(&TokenKind::Dot).is_some() {
                        if self.is_ident() {
                            let segment = self.consume_ident()?;
                            full_name = Text::from(format!("{}.{}", full_name, segment));
                        } else {
                            break;
                        }
                    }
                    List::from_iter([Ident::new(full_name, start_span)])
                } else {
                    // Zero-argument unfold: unfold all definitions
                    List::new()
                };
                Ok(TacticExpr::Unfold(names))
            }

            // Compute tactic
            Some(TokenKind::Ident(name))
                if name.as_str() == "compute" || name.as_str() == "norm_num" =>
            {
                self.stream.advance();
                Ok(TacticExpr::Compute)
            }

            // Try tactic: `try tactic` or `try { body } else { fallback }`
            Some(TokenKind::Try) => {
                self.stream.advance();
                let inner = self.parse_tactic_primary()?;
                if self.stream.consume(&TokenKind::Else).is_some() {
                    let fallback = self.parse_tactic_primary()?;
                    Ok(TacticExpr::TryElse {
                        body: Heap::new(inner),
                        fallback: Heap::new(fallback),
                    })
                } else {
                    Ok(TacticExpr::Try(Heap::new(inner)))
                }
            }

            // Repeat tactic
            Some(TokenKind::Ident(name)) if name.as_str() == "repeat" => {
                self.stream.advance();
                let inner = self.parse_tactic_primary()?;
                Ok(TacticExpr::Repeat(Heap::new(inner)))
            }

            // All goals tactic
            Some(TokenKind::Ident(name)) if name.as_str() == "all_goals" => {
                self.stream.advance();
                let inner = self.parse_tactic_primary()?;
                Ok(TacticExpr::AllGoals(Heap::new(inner)))
            }

            // Focus tactic
            Some(TokenKind::Ident(name)) if name.as_str() == "focus" => {
                self.stream.advance();
                let inner = self.parse_tactic_primary()?;
                Ok(TacticExpr::Focus(Heap::new(inner)))
            }

            // Done tactic
            Some(TokenKind::Ident(name)) if name.as_str() == "done" => {
                self.stream.advance();
                Ok(TacticExpr::Done)
            }

            // Admit/Sorry tactics
            Some(TokenKind::Ident(name)) if name.as_str() == "admit" => {
                self.stream.advance();
                Ok(TacticExpr::Admit)
            }
            Some(TokenKind::Ident(name)) if name.as_str() == "sorry" => {
                self.stream.advance();
                Ok(TacticExpr::Sorry)
            }

            // First/alternative tactics. Grammar allows two forms:
            //   `first [ t1, t2, t3 ]`  — comma-separated, matches Lean/Coq list syntax
            //   `first { t1; t2; t3 }`  — block form, matches verum.ebnf §2.19.7
            // Both desugar to TacticExpr::Alt(...).
            Some(TokenKind::Ident(name)) if name.as_str() == "first" => {
                self.stream.advance();
                if self.stream.check(&TokenKind::LBrace) {
                    self.stream.advance();
                    let mut tactics = Vec::new();
                    while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
                        tactics.push(self.parse_tactic_expr()?);
                        // Accept `;` or `,` as separator; trailing separator is allowed.
                        if self.stream.consume(&TokenKind::Semicolon).is_none() {
                            self.stream.consume(&TokenKind::Comma);
                        }
                    }
                    self.stream.expect(TokenKind::RBrace)?;
                    Ok(TacticExpr::Alt(tactics.into_iter().collect()))
                } else {
                    self.stream.expect(TokenKind::LBracket)?;
                    let tactics = self.comma_separated(|p| p.parse_tactic_expr())?;
                    self.stream.expect(TokenKind::RBracket)?;
                    Ok(TacticExpr::Alt(tactics.into_iter().collect()))
                }
            }

            // Named tactic (user-defined) - MUST be last for Ident matches
            Some(TokenKind::Ident(name)) => {
                let name_text = name.clone();
                let name_span = self.stream.current_span();
                self.stream.advance();

                // Optional generic arguments for polymorphic tactics:
                //   category_law<C>()
                //   functor_law<Identity>()
                //   category_law<F.Source>()
                //
                // We only commit to consuming the `<...>` when the parse
                // succeeds — otherwise we roll back so that tactic bodies
                // using `<` as a comparison operator keep working. Only
                // `Type::Type(...)` arguments are retained; const and
                // lifetime arguments are allowed by the underlying
                // parse_generic_args but are not meaningful for tactic
                // polymorphism yet, so they are silently dropped here.
                let generic_args: List<Type> = if self.stream.check(&TokenKind::Lt) {
                    let checkpoint = self.stream.position();
                    match self.parse_generic_args() {
                        Ok(args) => args
                            .into_iter()
                            .filter_map(|a| match a {
                                GenericArg::Type(t) => Some(t),
                                _ => None,
                            })
                            .collect(),
                        Err(_) => {
                            self.stream.reset_to(checkpoint);
                            List::new()
                        }
                    }
                } else {
                    List::new()
                };

                // Check for arguments
                let args = if self.stream.check(&TokenKind::LParen) {
                    self.stream.advance();
                    // Handle empty argument list: name()
                    let args = if self.stream.check(&TokenKind::RParen) {
                        Vec::new()
                    } else {
                        self.comma_separated(|p| p.parse_expr())?
                    };
                    self.stream.expect(TokenKind::RParen)?;
                    args.into_iter().collect()
                } else {
                    List::new()
                };

                Ok(TacticExpr::Named {
                    name: Ident::new(name_text, name_span),
                    args,
                    generic_args,
                })
            }

            // Grouped tactic: { t1; t2 }
            Some(TokenKind::LBrace) => {
                self.stream.advance();
                let mut tactics = Vec::new();
                while !self.stream.check(&TokenKind::RBrace) && !self.stream.at_end() {
                    tactics.push(self.parse_tactic_expr()?);
                    self.stream.consume(&TokenKind::Semicolon);
                }
                self.stream.expect(TokenKind::RBrace)?;
                Ok(TacticExpr::Seq(tactics.into_iter().collect()))
            }

            // Parenthesized tactic: ( t1; t2 )
            Some(TokenKind::LParen) => {
                self.stream.advance();
                let tactic = self.parse_tactic_expr()?;
                self.stream.expect(TokenKind::RParen)?;
                Ok(tactic)
            }

            // Keywords that can appear as tactic/justification names in proof context
            // E.g., `by definition`, `by lemma`, `by theorem`, `by precondition`
            Some(TokenKind::Lemma) | Some(TokenKind::Theorem) | Some(TokenKind::Axiom)
            | Some(TokenKind::Corollary) | Some(TokenKind::Match) | Some(TokenKind::Return) => {
                let name = match self.stream.peek_kind() {
                    Some(TokenKind::Lemma) => Text::from("lemma"),
                    Some(TokenKind::Theorem) => Text::from("theorem"),
                    Some(TokenKind::Axiom) => Text::from("axiom"),
                    Some(TokenKind::Corollary) => Text::from("corollary"),
                    Some(TokenKind::Match) => Text::from("match"),
                    Some(TokenKind::Return) => Text::from("return"),
                    _ => Text::from("_keyword"),
                };
                let span = self.stream.current_span();
                self.stream.advance();
                // Optional parenthesized arguments
                let args = if self.stream.check(&TokenKind::LParen) {
                    self.stream.advance();
                    let args = if self.stream.check(&TokenKind::RParen) {
                        Vec::new()
                    } else {
                        self.comma_separated(|p| p.parse_expr())?
                    };
                    self.stream.expect(TokenKind::RParen)?;
                    args.into_iter().collect()
                } else {
                    List::new()
                };
                Ok(TacticExpr::Named {
                    name: Ident::new(name, span),
                    args,
                    generic_args: List::new(),
                })
            }

            // Fallback: try to parse as an expression (for tactic arguments)
            _ => {
                if let Some(expr) = self.optional(|p| p.parse_expr()) {
                    Ok(TacticExpr::Named {
                        name: Ident::new(Text::from("_expr"), self.stream.current_span()),
                        args: List::from_iter(std::iter::once(expr)),
                        generic_args: List::new(),
                    })
                } else {
                    Err(ParseError::invalid_syntax(
                        "expected tactic expression",
                        self.stream.current_span(),
                    ))
                }
            }
        }
    }

    // ============================================================================
    // Helper Methods
    // ============================================================================

    /// Expect a contextual keyword (identifier with specific name).
    fn expect_contextual_keyword(&mut self, keyword: &str) -> ParseResult<()> {
        match self.stream.peek_kind() {
            Some(TokenKind::Ident(name)) if name.as_str() == keyword => {
                self.stream.advance();
                Ok(())
            }
            _ => Err(ParseError::invalid_syntax(
                format!("expected keyword '{}'", keyword),
                self.stream.current_span(),
            )),
        }
    }
}
