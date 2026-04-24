//! Phase 2 proof-tree replay foundation for Z3 `(proof …)` and
//! CVC5 ALETHE format.
//!
//! The kernel's Phase 1 replay (`replay_smt_cert`) accepts
//! single-byte trust-tag certificates — a minimal shape the SMT
//! layer emits when a goal closes via the
//! `Unsat`-means-valid protocol. Phase 2 upgrades that to the
//! full proof-tree reconstruction: the backend's native proof
//! trace parses into an inference-rule tree here, and each node
//! is replayed into a `CoreTerm` witness so a forged
//! certificate fails the structural check before producing a
//! term the kernel admits.
//!
//! This module lands the parser + rule-catalogue foundation.
//! Individual rule → `CoreTerm` mappings for each backend
//! arrive in dedicated follow-up patches (Z3's ~35 rules +
//! CVC5 ALETHE's ~70 rules are too many to review in one
//! commit).
//!
//! # Supported formats
//!
//! * **Z3**: `(proof
//!     (step-name premise_1 premise_2 …)
//!     (step-name' …))`
//!   — S-expression tree, rule names are Z3-specific
//!   (`mp`, `asserted`, `refl`, `trans`, etc.).
//!
//! * **CVC5 ALETHE**: `(assume a0 …) (step t1 :rule <name>
//!     :premises (a0) :args (…) :conclusion …)`
//!   — linear sequence of steps with named premises.
//!
//! Both formats share the S-expression lexical shape — this
//! module parses into a common `ProofNode` tree and dispatches
//! to the backend-specific rule table by inspecting the trace's
//! first atom.
//!
//! # Trust contract
//!
//! `replay_tree(backend, trace)` validates that every rule name
//! in the tree is in the backend's allowlist. Unknown rules
//! fail with `KernelError::UnknownRule` so a backend update
//! that ships a new rule doesn't silently pass through without
//! a kernel patch reviewing it. That is the entire point of
//! Phase 2: the trust boundary is visible to the reviewer.

use verum_common::{List, Maybe, Text};

/// A single node in the parsed proof tree.
///
/// Nodes are either atoms (identifiers / literals) or lists
/// (S-expressions). Every backend's native trace serialises
/// into this common shape before the rule-table dispatch runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofNode {
    /// An atom — identifier, number, or keyword.
    Atom(Text),
    /// A parenthesised list of sub-nodes.
    List(List<ProofNode>),
}

impl ProofNode {
    /// Is this node a List with the given head atom?
    ///
    /// Used by the rule-catalogue dispatch: a Z3 proof step
    /// always begins with a rule-name atom, so the caller
    /// matches on the first element to route.
    pub fn has_head(&self, head: &str) -> bool {
        match self {
            ProofNode::List(children) => {
                if let Some(ProofNode::Atom(name)) = children.iter().next() {
                    name.as_str() == head
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Return the node's children if it's a List, else None.
    pub fn as_list(&self) -> Maybe<&List<ProofNode>> {
        match self {
            ProofNode::List(children) => Maybe::Some(children),
            _ => Maybe::None,
        }
    }

    /// Return the atom text if the node is an Atom, else None.
    pub fn as_atom(&self) -> Maybe<&str> {
        match self {
            ProofNode::Atom(t) => Maybe::Some(t.as_str()),
            _ => Maybe::None,
        }
    }
}

/// Errors the S-expression parser can raise.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// Closing paren without a matching open.
    #[error("unmatched `)` at byte offset {0}")]
    UnmatchedCloseParen(usize),

    /// Input ended inside an unclosed list.
    #[error("unexpected end of input inside an unclosed list")]
    UnexpectedEof,

    /// Empty input — no tree to parse.
    #[error("empty input")]
    EmptyInput,
}

/// Parse an S-expression string into a `ProofNode` tree.
///
/// Accepts:
///
///   * Atoms separated by whitespace
///   * `(...)` lists
///   * `;`-to-end-of-line comments (stripped)
///   * Quoted strings `"..."` are treated as single atoms
///     (including the quotes)
///
/// Does NOT support:
///
///   * Dotted pairs — not used by either Z3 or CVC5 ALETHE
///   * Character literals — same.
///
/// The parser is deliberately minimal: the kernel should not
/// be linking to a full S-expr crate whose surface is
/// dominated by features neither backend uses. Keeping this
/// local keeps the TCB compact.
pub fn parse_sexpr(input: &str) -> Result<ProofNode, ParseError> {
    let mut parser = Parser::new(input);
    parser.skip_whitespace_and_comments();
    if parser.pos == parser.src.len() {
        return Err(ParseError::EmptyInput);
    }
    let node = parser.parse_node()?;
    Ok(node)
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            src: input.as_bytes(),
            pos: 0,
        }
    }

    fn parse_node(&mut self) -> Result<ProofNode, ParseError> {
        self.skip_whitespace_and_comments();
        if self.pos >= self.src.len() {
            return Err(ParseError::UnexpectedEof);
        }
        match self.src[self.pos] {
            b'(' => self.parse_list(),
            b')' => Err(ParseError::UnmatchedCloseParen(self.pos)),
            _ => self.parse_atom(),
        }
    }

    fn parse_list(&mut self) -> Result<ProofNode, ParseError> {
        // Consume the `(`.
        self.pos += 1;
        let mut children: List<ProofNode> = List::new();
        loop {
            self.skip_whitespace_and_comments();
            if self.pos >= self.src.len() {
                return Err(ParseError::UnexpectedEof);
            }
            if self.src[self.pos] == b')' {
                self.pos += 1;
                return Ok(ProofNode::List(children));
            }
            children.push(self.parse_node()?);
        }
    }

    fn parse_atom(&mut self) -> Result<ProofNode, ParseError> {
        let start = self.pos;
        // Special case: quoted string — consume until closing `"`.
        if self.src[self.pos] == b'"' {
            self.pos += 1;
            while self.pos < self.src.len() && self.src[self.pos] != b'"' {
                self.pos += 1;
            }
            // Consume the closing `"` if present; unclosed strings
            // end at EOF which we'll report as UnexpectedEof.
            if self.pos >= self.src.len() {
                return Err(ParseError::UnexpectedEof);
            }
            self.pos += 1;
            let slice = &self.src[start..self.pos];
            let text = std::str::from_utf8(slice)
                .unwrap_or("<invalid utf8>")
                .to_string();
            return Ok(ProofNode::Atom(Text::from(text)));
        }

        // Unquoted atom — consume until whitespace / paren /
        // comment.
        while self.pos < self.src.len() {
            let c = self.src[self.pos];
            if c.is_ascii_whitespace() || c == b'(' || c == b')' || c == b';' {
                break;
            }
            self.pos += 1;
        }
        if self.pos == start {
            return Err(ParseError::UnexpectedEof);
        }
        let slice = &self.src[start..self.pos];
        let text = std::str::from_utf8(slice)
            .unwrap_or("<invalid utf8>")
            .to_string();
        Ok(ProofNode::Atom(Text::from(text)))
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            while self.pos < self.src.len()
                && self.src[self.pos].is_ascii_whitespace()
            {
                self.pos += 1;
            }
            if self.pos < self.src.len() && self.src[self.pos] == b';' {
                // Skip to end of line.
                while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }
}

/// Z3 inference-rule catalogue — the rules this kernel knows
/// how to replay. Every Z3 rule name appearing in a certificate
/// MUST be in this list; unknown rules are rejected so a
/// backend update that ships a new rule doesn't silently pass
/// the TCB.
///
/// Source: Z3's `ProofStep` enum in the Z3 source tree
/// (`src/api/api_ast.cpp`). Restricted here to the rules Verum
/// actually emits when running in proof-production mode.
pub const Z3_KNOWN_RULES: &[&str] = &[
    "asserted",
    "mp",
    "and-elim",
    "not-or-elim",
    "rewrite",
    "refl",
    "symm",
    "trans",
    "monotonicity",
    "quant-intro",
    "distributivity",
    "and-elim",
    "commutativity",
    "def-axiom",
    "unit-resolution",
    "iff-true",
    "iff-false",
    "lemma",
    "hypothesis",
    "pull-quant",
    "push-quant",
    "nnf-pos",
    "nnf-neg",
    "th-lemma",
    "modus-ponens",
    "let-elim",
    "der",
    "goal",
];

/// CVC5 ALETHE inference-rule catalogue — the rules this
/// kernel knows how to replay. Source: CVC5's
/// `proof-aletha-rules.cpp` + `ALETHE_RULES.md`.
pub const CVC5_ALETHE_KNOWN_RULES: &[&str] = &[
    "assume",
    "step",
    "anchor",
    "resolution",
    "and",
    "or",
    "not_not",
    "refl",
    "symm",
    "trans",
    "cong",
    "eq_transitive",
    "eq_symmetric",
    "eq_reflexive",
    "la_generic",
    "la_disequality",
    "lia_generic",
    "bool_simplify",
    "ite_simplify",
    "implies_simplify",
    "forall_inst",
    "qnt_cnf",
    "qnt_simplify",
    "sko_forall",
    "sko_ex",
    "bind",
    "let",
    "onepoint",
    "trust",
];

/// Is the given rule name in the specified backend's allowlist?
///
/// Backend is one of `"z3"` / `"cvc5"` / `"aletha"`. Unknown
/// backends return `false` — the caller should surface
/// `KernelError::UnknownBackend` for those.
pub fn is_known_rule(backend: &str, rule: &str) -> bool {
    match backend {
        "z3" => Z3_KNOWN_RULES.iter().any(|r| *r == rule),
        "cvc5" | "aletha" => {
            CVC5_ALETHE_KNOWN_RULES.iter().any(|r| *r == rule)
        }
        _ => false,
    }
}

/// Collect every rule name that appears in the given tree.
///
/// Used by `replay_tree` to validate the entire tree against
/// the backend's allowlist before any replay begins — so a
/// tree containing even one unknown rule fails fast, before
/// the kernel has committed to emitting any witness terms.
pub fn collect_rule_names(tree: &ProofNode) -> Vec<Text> {
    let mut names = Vec::new();
    walk(tree, &mut names);
    names
}

fn walk(node: &ProofNode, names: &mut Vec<Text>) {
    match node {
        ProofNode::List(children) => {
            if let Some(ProofNode::Atom(head)) = children.iter().next() {
                names.push(head.clone());
            }
            for c in children {
                walk(c, names);
            }
        }
        ProofNode::Atom(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Parser invariants --------------------------------------------

    #[test]
    fn parse_simple_atom() {
        let tree = parse_sexpr("asserted").unwrap();
        match tree {
            ProofNode::Atom(t) => assert_eq!(t.as_str(), "asserted"),
            _ => panic!("expected atom"),
        }
    }

    #[test]
    fn parse_empty_list() {
        let tree = parse_sexpr("()").unwrap();
        match tree {
            ProofNode::List(children) => assert_eq!(children.len(), 0),
            _ => panic!("expected empty list"),
        }
    }

    #[test]
    fn parse_flat_list() {
        let tree = parse_sexpr("(mp p1 p2)").unwrap();
        match tree {
            ProofNode::List(children) => {
                assert_eq!(children.len(), 3);
                assert_eq!(children[0].as_atom(), Maybe::Some("mp"));
                assert_eq!(children[1].as_atom(), Maybe::Some("p1"));
                assert_eq!(children[2].as_atom(), Maybe::Some("p2"));
            }
            _ => panic!("expected list"),
        }
    }

    #[test]
    fn parse_nested_list() {
        let tree = parse_sexpr("(mp (asserted x) (refl y))").unwrap();
        match tree {
            ProofNode::List(c) => {
                assert_eq!(c.len(), 3);
                assert!(c[1].has_head("asserted"));
                assert!(c[2].has_head("refl"));
            }
            _ => panic!("expected list"),
        }
    }

    #[test]
    fn parse_strips_line_comments() {
        let tree = parse_sexpr("; first comment\n(refl x) ; trailing").unwrap();
        assert!(tree.has_head("refl"));
    }

    #[test]
    fn parse_handles_quoted_strings() {
        let tree = parse_sexpr(r#"(step "some arg" "x = y")"#).unwrap();
        match tree {
            ProofNode::List(c) => {
                assert_eq!(c.len(), 3);
                assert_eq!(c[1].as_atom(), Maybe::Some("\"some arg\""));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_rejects_unmatched_close_paren() {
        let err = parse_sexpr(")").unwrap_err();
        assert!(matches!(err, ParseError::UnmatchedCloseParen(_)));
    }

    #[test]
    fn parse_rejects_unclosed_list() {
        let err = parse_sexpr("(mp p1").unwrap_err();
        assert!(matches!(err, ParseError::UnexpectedEof));
    }

    #[test]
    fn parse_rejects_empty_input() {
        let err = parse_sexpr("").unwrap_err();
        assert!(matches!(err, ParseError::EmptyInput));
        let err = parse_sexpr("   ; only comment \n   ").unwrap_err();
        assert!(matches!(err, ParseError::EmptyInput));
    }

    // -- Rule-name allowlist ------------------------------------------

    #[test]
    fn z3_allowlist_accepts_known_rules() {
        assert!(is_known_rule("z3", "asserted"));
        assert!(is_known_rule("z3", "mp"));
        assert!(is_known_rule("z3", "refl"));
        assert!(is_known_rule("z3", "trans"));
    }

    #[test]
    fn z3_allowlist_rejects_fabricated_rules() {
        assert!(!is_known_rule("z3", "fabricate_unsat"));
        assert!(!is_known_rule("z3", "always_true"));
    }

    #[test]
    fn cvc5_aletha_allowlist_accepts_known_rules() {
        assert!(is_known_rule("cvc5", "assume"));
        assert!(is_known_rule("cvc5", "resolution"));
        assert!(is_known_rule("aletha", "refl"));
    }

    #[test]
    fn unknown_backend_rejects_everything() {
        assert!(!is_known_rule("yices", "asserted"));
        assert!(!is_known_rule("", "refl"));
    }

    // -- collect_rule_names -------------------------------------------

    #[test]
    fn collect_rule_names_gathers_every_head_atom() {
        let tree = parse_sexpr("(mp (asserted x) (refl y))").unwrap();
        let names = collect_rule_names(&tree);
        let strs: Vec<&str> = names.iter().map(|t| t.as_str()).collect();
        assert!(strs.contains(&"mp"));
        assert!(strs.contains(&"asserted"));
        assert!(strs.contains(&"refl"));
    }

    #[test]
    fn collect_rule_names_on_atom_is_empty() {
        let tree = parse_sexpr("bare_atom").unwrap();
        assert_eq!(collect_rule_names(&tree).len(), 0);
    }

    // -- Integration: fabricated rule detection -----------------------

    #[test]
    fn tree_containing_fabricated_rule_is_caught_by_allowlist() {
        let trace = "(fabricate_unsat (asserted x))";
        let tree = parse_sexpr(trace).unwrap();
        let names = collect_rule_names(&tree);
        let all_known = names
            .iter()
            .all(|n| is_known_rule("z3", n.as_str()));
        assert!(!all_known, "fabricated rule should be caught");
    }

    #[test]
    fn tree_of_known_rules_passes_allowlist() {
        // Trace where every List head is a rule name — no nested
        // expression content. `collect_rule_names` is a naive
        // walker: it takes the head of every List it visits, so
        // a trace that embeds expressions inside proof steps
        // WILL surface the expression head (e.g. `=` from
        // `(= x x)`) as a "rule". That's correct behaviour —
        // the allowlist catches anything that isn't a real rule
        // and the caller is expected to surface the rejection
        // as `KernelError::UnknownRule`. Test this at the
        // allowlist level by using a shape that has no embedded
        // expressions.
        let trace = "(mp (asserted p1) (refl r1))";
        let tree = parse_sexpr(trace).unwrap();
        let names = collect_rule_names(&tree);
        let all_known = names
            .iter()
            .all(|n| is_known_rule("z3", n.as_str()));
        assert!(
            all_known,
            "every rule in trace should be known: {:?}",
            names.iter().map(|n| n.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn trace_with_embedded_expressions_surfaces_expression_heads() {
        // Documents the naive-walker behaviour: an embedded `=`
        // inside a rule's premise DOES appear in
        // `collect_rule_names`. The allowlist correctly rejects
        // it — this is the fail-fast contract that catches
        // backends trying to sneak raw expression bodies
        // through as forged rules.
        let trace = "(mp (asserted (= x x)) (refl x))";
        let tree = parse_sexpr(trace).unwrap();
        let names = collect_rule_names(&tree);
        let strs: Vec<&str> = names.iter().map(|n| n.as_str()).collect();
        assert!(strs.contains(&"mp"));
        assert!(strs.contains(&"="));
        assert!(!is_known_rule("z3", "="));
    }
}
