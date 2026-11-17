//! SMT proof-tree syntax and diagnostic rule catalogues.
//!
//! Parsing and recognizing names does not reconstruct inference semantics.
//! `replay_z3_tree` and `replay_aletha_tree` preserve bounded structural
//! diagnostics and refuse recognized trees as unsupported evidence. They do
//! not create axioms or Boolean witnesses. The typed proof graph below records
//! Verum's own inference rules separately from untrusted solver syntax.

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
///  * Atoms separated by whitespace
///  * `(...)` lists
///  * `;`-to-end-of-line comments (stripped)
///  * Quoted strings `"..."` are treated as single atoms
///  (including the quotes)
///
/// Does NOT support:
///
///  * Dotted pairs — not used by either Z3 or CVC5 ALETHE
///  * Character literals — same.
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
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_whitespace() {
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
        "z3" => Z3_KNOWN_RULES.contains(&rule),
        "cvc5" | "aletha" => CVC5_ALETHE_KNOWN_RULES.contains(&rule),
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

// =============================================================================
// Phase 2 replay — rule → CoreTerm witness construction
// =============================================================================

use crate::{CoreTerm, KernelError};

#[derive(Clone, Copy)]
enum TreeDialect {
    Z3,
    Alethe,
}

impl TreeDialect {
    fn backend(self) -> &'static str {
        match self {
            Self::Z3 => "z3",
            Self::Alethe => "aletha",
        }
    }
}

/// Inspect structural solver evidence, never infer a theorem from rule names.
/// All nodes, including ALETHE expression payloads, consume shared verifier
/// budgets. Lists in Z3's legacy diagnostic shape are treated as rule nodes;
/// ALETHE expression/premise payloads are not semantic derivations.
fn inspect_proof_tree(tree: &ProofNode, dialect: TreeDialect) -> Result<Text, KernelError> {
    use verum_proof::limits::AdmissionLimits;
    let limits = AdmissionLimits::DEFAULT;
    let exhausted = |resource, limit| KernelError::SmtProofResourceLimit { resource, limit };
    let mut pending = List::new();
    pending.push((tree, 0usize, true));
    let mut scheduled = 1usize;
    let mut bytes = 0usize;
    let mut root_rule = None;
    while let Some((node, depth, is_rule)) = pending.pop() {
        match node {
            ProofNode::Atom(atom) => {
                if is_rule {
                    return Err(KernelError::SmtReplayFailed {
                        reason: Text::from("proof tree must be a list, got a bare atom"),
                    });
                }
                bytes = bytes
                    .checked_add(atom.len())
                    .filter(|n| *n <= limits.max_bytes)
                    .ok_or_else(|| exhausted("bytes", limits.max_bytes))?;
            }
            ProofNode::List(children) => {
                // Bound the list before scanning ALETHE keyword/value pairs.
                scheduled = scheduled
                    .checked_add(children.len())
                    .filter(|n| *n <= limits.max_nodes)
                    .ok_or_else(|| exhausted("nodes", limits.max_nodes))?;
                if !children.is_empty() && depth >= limits.max_depth {
                    return Err(exhausted("depth", limits.max_depth));
                }
                if is_rule {
                    let Some(ProofNode::Atom(head)) = children.iter().next() else {
                        return Err(KernelError::SmtReplayFailed {
                            reason: Text::from("proof-tree list starts with a non-atom head"),
                        });
                    };
                    let rule = match dialect {
                        TreeDialect::Alethe if head.as_str() == "step" => {
                            extract_aletha_rule_name(children).ok_or_else(|| {
                                KernelError::SmtReplayFailed {
                                    reason: Text::from("ALETHE step missing :rule keyword"),
                                }
                            })?
                        }
                        _ => head,
                    };
                    if !is_known_rule(dialect.backend(), rule.as_str()) {
                        return Err(KernelError::UnknownRule {
                            backend: Text::from(dialect.backend()),
                            tag: 0,
                        });
                    }
                    if root_rule.is_none() {
                        root_rule = Some(rule.clone());
                    }
                }
                for child in children.iter().rev() {
                    let child_is_rule = is_rule
                        && match child {
                            ProofNode::Atom(_) => false,
                            ProofNode::List(nested) => match dialect {
                                TreeDialect::Z3 => true,
                                TreeDialect::Alethe => {
                                    matches!(nested.iter().next(), Some(ProofNode::Atom(head))
                                if head.as_str() == "step" || is_known_rule(dialect.backend(), head.as_str()))
                                }
                            },
                        };
                    pending.push((child, depth + 1, child_is_rule));
                }
            }
        }
    }
    root_rule.ok_or_else(|| KernelError::SmtReplayFailed {
        reason: Text::from("proof tree has no rule"),
    })
}

/// Opaque Z3 proof trees are diagnostic evidence. Recognized structure is
/// refused until each rule has an independently checked reconstruction.
pub fn replay_z3_tree(tree: &ProofNode) -> Result<CoreTerm, KernelError> {
    let rule = inspect_proof_tree(tree, TreeDialect::Z3)?;
    Err(KernelError::UnsupportedSmtProofTree {
        backend: Text::from("z3"),
        rule,
    })
}

/// ALETHE rule names and premise references do not establish an obligation.
/// Preserve structural diagnostics, but never fabricate framework axioms.
pub fn replay_aletha_tree(tree: &ProofNode) -> Result<CoreTerm, KernelError> {
    let rule = inspect_proof_tree(tree, TreeDialect::Alethe)?;
    Err(KernelError::UnsupportedSmtProofTree {
        backend: Text::from("aletha"),
        rule,
    })
}

/// Extract the diagnostic rule name after `:rule`, borrowing the input.
fn extract_aletha_rule_name(children: &List<ProofNode>) -> Option<&Text> {
    children
        .iter()
        .zip(children.iter().skip(1))
        .find_map(|(key, value)| match (key, value) {
            (ProofNode::Atom(key), ProofNode::Atom(value)) if key.as_str() == ":rule" => {
                Some(value)
            }
            _ => None,
        })
}

// =============================================================================
// Kernel-rule typed proof-graph surface
// =============================================================================
//

// The S-expression-based ProofNode above models *backend* proof
// trees (Z3, CVC5 ALETHE). introduces a parallel,
// typed surface that captures the kernel's OWN inference-rule
// applications when typing a CoreTerm — the typing-derivation
// graph that feeds:
//

//  • verum audit --proof-trace (TCB enumeration per theorem)
//  • Certificate export to Lean/Coq/Agda (trace-to-tactic-script
//  reconstruction)
//  • IDE step-debugger (interactive proof exploration)
//  • Cross-tool replay matrix (#90)

/// kernel inference rule taxonomy. One variant per
/// shipped typing rule per `verification-architecture.md` §4.4a.
///
/// The `Display` representation is the canonical short name
/// (`"K-App"`, `"K-Refine-omega"`, etc.) used by audit output and
/// certificate-export targets.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KernelRule {
    // Structural (CCHM core)
    /// Variable lookup: `Γ, x:A ⊢ x : A`.
    KVar,
    /// Universe formation: `Γ ⊢ U_n : U_{n+1}`.
    KUniv,
    /// Π-type formation: dependent function type.
    KPiForm,
    /// λ-introduction: form an anonymous function.
    KLamIntro,
    /// Application elimination: apply a function to an argument.
    KAppElim,
    /// Σ-type formation: dependent pair type.
    KSigmaForm,
    /// Pair introduction: form a Σ-pair `(a, b)`.
    KPairIntro,
    /// First projection from a Σ-pair.
    KFstElim,
    /// Second projection from a Σ-pair.
    KSndElim,
    // Cubical
    /// Path-type formation `Path<A>(a, b)`.
    KPathTyForm,
    /// Dependent path-over `PathOver(motive, p, lhs, rhs) : U`.
    KPathOverForm,
    /// Reflexivity introduction `refl(a) : Path<A>(a, a)`.
    KReflIntro,
    /// Cubical homogeneous composition `hcomp φ walls base`.
    KHComp,
    /// Ordinary path transport `Transport(path, value)` (stable rule ID).
    KTransp,
    /// Glue type `Glue<A>(φ, T, e)` for computational univalence.
    KGlue,
    // Refinement
    /// Refinement-type formation `{ x : A | P(x) }` (finite-depth).
    KRefine,
    /// Ordinal-depth-stratified refinement formation
    /// (modal-depth gating, transfinite version of K-Refine).
    KRefineOmega,
    /// Refinement introduction (witness a refinement type).
    KRefineIntro,
    /// Refinement erasure (drop the predicate, keep the carrier).
    KRefineErase,
    // Quotient types
    /// Quotient-type formation `Quotient(T, ~)`.
    KQuotForm,
    /// Quotient introduction `[t]_~ : T / ~`.
    KQuotIntro,
    /// Quotient elimination with motive + respect-of-equivalence.
    KQuotElim,
    // Inductive
    /// Inductive-type registration with strict-positivity guard.
    KInductive,
    /// Strict-positivity check (Berardi 1998 paradox-immunity).
    KPos,
    /// Generic eliminator dispatch for inductive constructors.
    KElim,
    // SMT + Axiom
    /// SMT-certificate replay through the trusted-base proof checker.
    KSmt,
    /// Framework-axiom registration with subsingleton + body-is-Prop
    /// gates against UIP-shape leakage.
    KFwAx,
    // Diakrisis verification-architecture
    /// Naturality-square `K-Eps-Mu`: `ε∘M ≃ A∘ε` coherence
    /// (Proposition 5.1 / Corollary 5.10).
    KEpsMu,
    /// Universe-ascent admission for `M_stack(α) : Articulation@U_{k+1}`.
    KUniverseAscent,
    /// `K-Round-Trip` admission rule for the OC/DC duality
    /// round-trip (Theorem 108.T / Theorem 16.10). Premise:
    /// `α : Articulation` and `α.is_finitely_axiomatized()`;
    /// conclusion: `RoundTripCert{α} : Type` is admissible.
    /// ships the kernel-taxonomy entry without the algorithmic
    /// canonicalize check; V1 adds the
    /// `canonicalize(inverse(translate(α))) = canonicalize(α)`
    /// equality check.
    KRoundTrip,
    /// Type-formation for the meta-classifier `ε(α)`.
    KEpsilonOf,
    /// Type-formation for the OC-coordinate `α(ε)`.
    KAlphaOf,
    /// Necessity modality `□A` (S4 / S5 frames per pragma).
    KModalBox,
    /// Possibility modality `◇A` (S4 / S5 frames per pragma).
    KModalDiamond,
    /// Indexed conjunction `⋀_i A_i` over a modal index set.
    KModalBigAnd,
    /// cohesive shape modality `∫A` (Schreiber DCCT).
    KShape,
    /// cohesive flat modality `♭A` (Schreiber DCCT).
    KFlat,
    /// cohesive sharp modality `♯A` (Schreiber DCCT).
    KSharp,
}

/// Canonical metadata for a single [`KernelRule`].
///
/// The three fields used to live as three parallel methods on
/// [`KernelRule`] (`name()` / `v_stage()` / `category()`), each with
/// its own 38-arm match expression — a drift-prone shape: adding a
/// new rule meant remembering to touch all three places, with the
/// compiler only flagging the missing `name` and `v_stage` arms (the
/// `category()` match used grouped patterns + `_` fallthrough). The
/// metadata now lives in one [`KernelRule::meta`] match per rule, and
/// the three accessor methods are trivial projections over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KernelRuleMeta {
    /// Canonical short name (used by audit + export targets), e.g.
    /// `"K-App-Elim"`, `"K-Refine-omega"`, `"K-PathOver-Form"`.
    pub name: &'static str,
    /// V-stage maturity tag per spec §4.4a.7 (e.g. `"V0"`, `"V8"`,
    /// `"V8.1"`). Returned as a stable string for audit output.
    pub v_stage: &'static str,
    /// Abstract semantic category for `verum audit --kernel-rules`
    /// grouping. `None` for rules that fit no specific category
    /// (e.g. variable-binding rules universal across every
    /// dependent type theory).
    pub category: Option<&'static str>,
}

impl KernelRule {
    /// Per-rule metadata: canonical name, V-stage tag, and audit
    /// category. **Single source of truth** — the [`Self::name`],
    /// [`Self::v_stage`], and [`Self::category`] accessor methods
    /// are one-liners over this struct, so adding a new rule to
    /// the enum requires extending exactly one match arm here.
    pub fn meta(&self) -> KernelRuleMeta {
        // Helper closure to keep each arm a single line.
        // Order mirrors the enum declaration so a reviewer can read
        // both top-to-bottom in lock-step.
        let m = |name, v_stage, category| KernelRuleMeta {
            name,
            v_stage,
            category,
        };
        const CORE: Option<&'static str> = Some("Dependent type theory — core");
        const CUBICAL: Option<&'static str> = Some("Cubical type theory");
        const QUOTIENT: Option<&'static str> = Some("Quotient types");
        const REFINEMENT: Option<&'static str> = Some("Refinement subtyping");
        const COHESIVE: Option<&'static str> = Some("Cohesive modalities");
        const MODAL: Option<&'static str> = Some("Modal logic operators");
        const EPS_ALPHA: Option<&'static str> = Some("ε / α modality duality");
        match self {
            // -- Structural (CCHM core) --
            KernelRule::KVar => m("K-Var", "V0", None),
            KernelRule::KUniv => m("K-Univ", "V8", CORE),
            KernelRule::KPiForm => m("K-Pi-Form", "V0", CORE),
            KernelRule::KLamIntro => m("K-Lam-Intro", "V0", CORE),
            KernelRule::KAppElim => m("K-App-Elim", "V8", CORE),
            KernelRule::KSigmaForm => m("K-Sigma-Form", "V0", CORE),
            KernelRule::KPairIntro => m("K-Pair-Intro", "V0", CORE),
            KernelRule::KFstElim => m("K-Fst-Elim", "V0", CORE),
            KernelRule::KSndElim => m("K-Snd-Elim", "V0", CORE),
            // -- Cubical --
            KernelRule::KPathTyForm => m("K-PathTy-Form", "V8", CUBICAL),
            KernelRule::KPathOverForm => m(
                "K-PathOver-Form",
                "V8.1",
                Some("Higher inductive types — dependent path-over"),
            ),
            KernelRule::KReflIntro => m("K-Refl-Intro", "V0", CUBICAL),
            KernelRule::KHComp => m("K-HComp", "V0", CUBICAL),
            KernelRule::KTransp => m("K-Transp", "V0", CUBICAL),
            KernelRule::KGlue => m("K-Glue", "V0", CUBICAL),
            // -- Refinement --
            KernelRule::KRefine => m("K-Refine", "V0", REFINEMENT),
            KernelRule::KRefineOmega => m(
                "K-Refine-omega",
                "V8",
                Some("Refinement subtyping — transfinite modal-depth"),
            ),
            KernelRule::KRefineIntro => m("K-Refine-Intro", "V0", REFINEMENT),
            KernelRule::KRefineErase => m("K-Refine-Erase", "V0", REFINEMENT),
            // -- Quotient --
            KernelRule::KQuotForm => m("K-Quot-Form", "V8", QUOTIENT),
            KernelRule::KQuotIntro => m("K-Quot-Intro", "V8", QUOTIENT),
            KernelRule::KQuotElim => m("K-Quot-Elim", "V8", QUOTIENT),
            // -- Inductive --
            KernelRule::KInductive => m("K-Inductive", "V8", Some("Inductive type formation")),
            KernelRule::KPos => m("K-Pos", "V0", Some("Inductive type — strict positivity")),
            KernelRule::KElim => m("K-Elim", "V8", Some("Inductive type — eliminator")),
            // -- SMT + Axiom --
            KernelRule::KSmt => m("K-Smt", "V8", Some("SMT proof replay")),
            KernelRule::KFwAx => m("K-FwAx", "V8", Some("Framework axiom admission")),
            // -- Diakrisis verification-architecture --
            KernelRule::KEpsMu => m(
                "K-Eps-Mu",
                "V2",
                Some("ε / α modality duality — naturality"),
            ),
            KernelRule::KUniverseAscent => m(
                "K-Universe-Ascent",
                "V1",
                Some("Stratified universe ascent"),
            ),
            KernelRule::KRoundTrip => {
                m("K-Round-Trip", "V0", Some("OC / DC translation round-trip"))
            }
            KernelRule::KEpsilonOf => m("K-EpsilonOf", "V0", EPS_ALPHA),
            KernelRule::KAlphaOf => m("K-AlphaOf", "V0", EPS_ALPHA),
            KernelRule::KModalBox => m("K-ModalBox", "V0", MODAL),
            KernelRule::KModalDiamond => m("K-ModalDiamond", "V0", MODAL),
            KernelRule::KModalBigAnd => m("K-ModalBigAnd", "V0", MODAL),
            // -- Cohesive modalities ∫ ⊣ ♭ ⊣ ♯ --
            KernelRule::KShape => m("K-Shape", "V8", COHESIVE),
            KernelRule::KFlat => m("K-Flat", "V8", COHESIVE),
            KernelRule::KSharp => m("K-Sharp", "V8", COHESIVE),
        }
    }

    /// Canonical short name (used by audit + export targets).
    /// Thin accessor over [`Self::meta`].
    #[inline]
    pub fn name(&self) -> &'static str {
        self.meta().name
    }

    /// V-stage maturity tag per spec §4.4a.7. Returned as a
    /// stable string for audit output (e.g. `"V0"`, `"V8"`).
    /// Thin accessor over [`Self::meta`].
    #[inline]
    pub fn v_stage(&self) -> &'static str {
        self.meta().v_stage
    }

    /// Abstract semantic category this rule belongs to.
    ///
    /// Returns a short, generic label naming the *kind* of rule
    /// (e.g. `"Quotient types"`, `"Cohesive modalities"`,
    /// `"Higher inductive types"`) without reference to any
    /// internal specification document or ticket number. Used by
    /// `verum audit --kernel-rules` to group rules by domain in
    /// the audit report.
    ///
    /// Returns `None` for rules that fit no specific category
    /// (e.g. variable-binding rules that are universal across
    /// every dependent type theory). Thin accessor over
    /// [`Self::meta`].
    #[inline]
    pub fn category(&self) -> Option<&'static str> {
        self.meta().category
    }

    /// Iterate every kernel-rule variant in canonical declaration
    /// order. Used by audit + drift-check infrastructure to walk
    /// the full taxonomy without re-listing the variants by hand.
    ///
    /// **Invariant**: the array length equals the number of
    /// variants in [`KernelRule`]. The pin test
    /// `kernel_rule_full_list_covers_every_variant` enforces this
    /// — adding a new variant without updating this array is a
    /// load-bearing omission and surfaces as a test failure.
    pub fn full_list() -> &'static [KernelRule] {
        &[
            // Structural
            KernelRule::KVar,
            KernelRule::KUniv,
            KernelRule::KPiForm,
            KernelRule::KLamIntro,
            KernelRule::KAppElim,
            KernelRule::KSigmaForm,
            KernelRule::KPairIntro,
            KernelRule::KFstElim,
            KernelRule::KSndElim,
            // Cubical
            KernelRule::KPathTyForm,
            KernelRule::KPathOverForm,
            KernelRule::KReflIntro,
            KernelRule::KHComp,
            KernelRule::KTransp,
            KernelRule::KGlue,
            // Refinement
            KernelRule::KRefine,
            KernelRule::KRefineOmega,
            KernelRule::KRefineIntro,
            KernelRule::KRefineErase,
            // Quotient
            KernelRule::KQuotForm,
            KernelRule::KQuotIntro,
            KernelRule::KQuotElim,
            // Inductive
            KernelRule::KInductive,
            KernelRule::KPos,
            KernelRule::KElim,
            // SMT + Axiom
            KernelRule::KSmt,
            KernelRule::KFwAx,
            // Diakrisis verification-architecture
            KernelRule::KEpsMu,
            KernelRule::KUniverseAscent,
            KernelRule::KRoundTrip,
            KernelRule::KEpsilonOf,
            KernelRule::KAlphaOf,
            KernelRule::KModalBox,
            KernelRule::KModalDiamond,
            KernelRule::KModalBigAnd,
            // Cohesive
            KernelRule::KShape,
            KernelRule::KFlat,
            KernelRule::KSharp,
        ]
    }
}

impl std::fmt::Display for KernelRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// one node in the kernel-rule proof graph.
///
/// Each node records:
///  • The **rule** that justified the inference (e.g.
///  `KernelRule::KAppElim`).
///  • The **conclusion** — the CoreTerm asserted as well-typed
///  under the rule.
///  • The **inferred type** — what the kernel computed for the
///  conclusion under that rule.
///  • The **premises** — sub-derivations of the rule's
///  hypothesis terms.
///  • A **citation** when the rule references an external
///  framework (K-FwAx pulls in the FrameworkId).
///
/// Together these form an LCF-style proof tree: a closed,
/// re-checkable representation of the typing derivation.
#[derive(Debug, Clone, PartialEq)]
pub struct KernelProofNode {
    /// Inference rule applied at this node.
    pub rule: KernelRule,
    /// The term whose typing this node proves.
    pub conclusion: CoreTerm,
    /// The kernel-inferred type of the conclusion.
    pub inferred_ty: CoreTerm,
    /// Sub-derivations for the rule's premises (boxed via Heap
    /// to keep the tree allocation-clean).
    pub premises: List<KernelProofNode>,
    /// Optional framework citation for K-FwAx / K-SmtRecheck
    /// nodes; `None` for the structural / cubical / refinement
    /// rules whose justification is purely intrinsic.
    pub citation: verum_common::Maybe<crate::FrameworkId>,
}

impl KernelProofNode {
    /// Construct a leaf node — no premises, intrinsic-rule
    /// justification.
    pub fn leaf(rule: KernelRule, conclusion: CoreTerm, inferred_ty: CoreTerm) -> Self {
        Self {
            rule,
            conclusion,
            inferred_ty,
            premises: List::new(),
            citation: verum_common::Maybe::None,
        }
    }

    /// Construct a node with explicit premises.
    pub fn with_premises(
        rule: KernelRule,
        conclusion: CoreTerm,
        inferred_ty: CoreTerm,
        premises: List<KernelProofNode>,
    ) -> Self {
        Self {
            rule,
            conclusion,
            inferred_ty,
            premises,
            citation: verum_common::Maybe::None,
        }
    }

    /// Attach a framework citation. Returns `self` builder-style.
    pub fn with_citation(mut self, citation: crate::FrameworkId) -> Self {
        self.citation = verum_common::Maybe::Some(citation);
        self
    }

    /// Walk the proof tree depth-first and call `visit` for every
    /// node (including this one). Used by audit / export
    /// machinery that needs to enumerate every rule application.
    pub fn walk_dfs<F: FnMut(&KernelProofNode)>(&self, visit: &mut F) {
        visit(self);
        for p in self.premises.iter() {
            p.walk_dfs(visit);
        }
    }

    /// Total number of nodes in the proof tree (including this
    /// one). Useful for size-bounded audit output.
    pub fn size(&self) -> usize {
        let mut n = 0;
        self.walk_dfs(&mut |_| {
            n += 1;
        });
        n
    }

    /// Collect every distinct [`KernelRule`] that appears in the
    /// tree. Used by `verum audit --proof-trace` to enumerate
    /// the rules a theorem's proof depends on.
    pub fn rules_used(&self) -> std::collections::BTreeSet<KernelRule> {
        let mut out = std::collections::BTreeSet::new();
        self.walk_dfs(&mut |node| {
            // KernelRule doesn't impl Ord directly (PartialEq +
            // Hash only) — go through the canonical name string.
            // BTreeSet<KernelRule> would need Ord; track names
            // for sorted output and bridge back when needed.
            // For now, the BTreeSet is keyed on KernelRule via
            // a manual Ord impl.
            out.insert(node.rule.clone());
        });
        out
    }
}

// Provide Ord on KernelRule so it can live in BTreeSet/BTreeMap
// for deterministic audit-output ordering.
impl PartialOrd for KernelRule {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for KernelRule {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name().cmp(other.name())
    }
}

/// reconstruct the kernel proof tree for `term` by
/// walking its CoreTerm structure and synthesising the
/// inference-rule applications post-hoc.
///
/// This is a **best-effort** typing-derivation tracer:
///
///  • Supported constructors map to a [`KernelRule`]. Registered inductive
///  constructors require declaration-aware tracing and return `None` here.
///  • For composite constructors (App / Pair / Refl / etc.)
///  the function recurses to build premise sub-trees.
///  • The conclusion is the term being typed; the inferred_ty
///  is the result of [`crate::infer()`] on it.
///  • If `infer` fails for the term or any sub-term, the
///  reconstruction returns `None` rather than a partial
///  tree (the caller can re-run `infer` to get the precise
///  error).
///
/// This is diagnostic structure, not independent proof replay. In particular,
/// it does not certify the registered-inductive checking API.
pub fn record_inference(
    ctx: &crate::Context,
    term: &CoreTerm,
    axioms: &crate::AxiomRegistry,
) -> Option<KernelProofNode> {
    let inferred_ty = match crate::infer(ctx, term, axioms) {
        Ok(t) => t,
        Err(_) => return None,
    };
    let (rule, premises) = inference_rule_and_premises(ctx, term, axioms)?;
    Some(KernelProofNode {
        rule,
        conclusion: term.clone(),
        inferred_ty,
        premises,
        citation: match term {
            CoreTerm::Axiom { framework, .. } => verum_common::Maybe::Some(framework.clone()),
            _ => verum_common::Maybe::None,
        },
    })
}

/// Map a CoreTerm constructor to its kernel rule + premise
/// sub-derivations. Premises are recursively built via
/// [`record_inference`] for sub-terms; on failure the premise
/// list is truncated (the caller can re-run `infer` for the
/// precise error site).
fn inference_rule_and_premises(
    ctx: &crate::Context,
    term: &CoreTerm,
    axioms: &crate::AxiomRegistry,
) -> Option<(KernelRule, List<KernelProofNode>)> {
    let mut premises: List<KernelProofNode> = List::new();
    let rule = match term {
        CoreTerm::Var(_) => KernelRule::KVar,
        // This registry-free tracing API cannot reconstruct an inductive
        // declaration's derivation. It must not label a constructor as a
        // local-variable rule or claim external replay coverage.
        CoreTerm::Constructor { .. } => return None,
        CoreTerm::Universe(_) => KernelRule::KUniv,
        CoreTerm::Pi {
            domain, codomain, ..
        } => {
            if let Some(p) = record_inference(ctx, domain, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, codomain, axioms) {
                premises.push(p);
            }
            KernelRule::KPiForm
        }
        CoreTerm::Lam { domain, body, .. } => {
            if let Some(p) = record_inference(ctx, domain, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, body, axioms) {
                premises.push(p);
            }
            KernelRule::KLamIntro
        }
        CoreTerm::App(f, arg) => {
            if let Some(p) = record_inference(ctx, f, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, arg, axioms) {
                premises.push(p);
            }
            KernelRule::KAppElim
        }
        CoreTerm::Sigma { fst_ty, snd_ty, .. } => {
            if let Some(p) = record_inference(ctx, fst_ty, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, snd_ty, axioms) {
                premises.push(p);
            }
            KernelRule::KSigmaForm
        }
        CoreTerm::Pair(a, b) => {
            if let Some(p) = record_inference(ctx, a, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, b, axioms) {
                premises.push(p);
            }
            KernelRule::KPairIntro
        }
        CoreTerm::Fst(p_inner) => {
            if let Some(p) = record_inference(ctx, p_inner, axioms) {
                premises.push(p);
            }
            KernelRule::KFstElim
        }
        CoreTerm::Snd(p_inner) => {
            if let Some(p) = record_inference(ctx, p_inner, axioms) {
                premises.push(p);
            }
            KernelRule::KSndElim
        }
        CoreTerm::PathTy { carrier, lhs, rhs } => {
            if let Some(p) = record_inference(ctx, carrier, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, lhs, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, rhs, axioms) {
                premises.push(p);
            }
            KernelRule::KPathTyForm
        }
        CoreTerm::PathOver {
            motive,
            path,
            lhs,
            rhs,
        } => {
            if let Some(p) = record_inference(ctx, motive, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, path, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, lhs, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, rhs, axioms) {
                premises.push(p);
            }
            KernelRule::KPathOverForm
        }
        CoreTerm::Refl(x) => {
            if let Some(p) = record_inference(ctx, x, axioms) {
                premises.push(p);
            }
            KernelRule::KReflIntro
        }
        CoreTerm::HComp { phi, walls, base } => {
            if let Some(p) = record_inference(ctx, phi, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, walls, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, base, axioms) {
                premises.push(p);
            }
            KernelRule::KHComp
        }
        CoreTerm::Transport { path, value } => {
            if let Some(p) = record_inference(ctx, path, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, value, axioms) {
                premises.push(p);
            }
            KernelRule::KTransp
        }
        CoreTerm::Glue {
            carrier,
            phi,
            fiber,
            equiv,
        } => {
            if let Some(p) = record_inference(ctx, carrier, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, phi, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, fiber, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, equiv, axioms) {
                premises.push(p);
            }
            KernelRule::KGlue
        }
        CoreTerm::Refine {
            base, predicate, ..
        } => {
            if let Some(p) = record_inference(ctx, base, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, predicate, axioms) {
                premises.push(p);
            }
            KernelRule::KRefine
        }
        CoreTerm::Quotient { base, equiv } => {
            if let Some(p) = record_inference(ctx, base, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, equiv, axioms) {
                premises.push(p);
            }
            KernelRule::KQuotForm
        }
        CoreTerm::QuotIntro { value, base, equiv } => {
            if let Some(p) = record_inference(ctx, value, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, base, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, equiv, axioms) {
                premises.push(p);
            }
            KernelRule::KQuotIntro
        }
        CoreTerm::QuotElim {
            scrutinee,
            motive,
            case,
            respect,
        } => {
            if let Some(p) = record_inference(ctx, scrutinee, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, motive, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, case, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, respect, axioms) {
                premises.push(p);
            }
            KernelRule::KQuotElim
        }
        CoreTerm::Inductive { args, .. } => {
            for a in args.iter() {
                if let Some(p) = record_inference(ctx, a, axioms) {
                    premises.push(p);
                }
            }
            KernelRule::KInductive
        }
        CoreTerm::Elim {
            scrutinee,
            motive,
            cases,
        } => {
            if let Some(p) = record_inference(ctx, scrutinee, axioms) {
                premises.push(p);
            }
            if let Some(p) = record_inference(ctx, motive, axioms) {
                premises.push(p);
            }
            for c in cases.iter() {
                if let Some(p) = record_inference(ctx, c, axioms) {
                    premises.push(p);
                }
            }
            KernelRule::KElim
        }
        CoreTerm::SmtProof(_) => KernelRule::KSmt,
        CoreTerm::Axiom { ty, .. } => {
            // The axiom's body type is itself a sub-derivation.
            if let Some(p) = record_inference(ctx, ty, axioms) {
                premises.push(p);
            }
            KernelRule::KFwAx
        }
        CoreTerm::EpsilonOf(t) => {
            if let Some(p) = record_inference(ctx, t, axioms) {
                premises.push(p);
            }
            KernelRule::KEpsilonOf
        }
        CoreTerm::AlphaOf(t) => {
            if let Some(p) = record_inference(ctx, t, axioms) {
                premises.push(p);
            }
            KernelRule::KAlphaOf
        }
        CoreTerm::ModalBox(t) => {
            if let Some(p) = record_inference(ctx, t, axioms) {
                premises.push(p);
            }
            KernelRule::KModalBox
        }
        CoreTerm::ModalDiamond(t) => {
            if let Some(p) = record_inference(ctx, t, axioms) {
                premises.push(p);
            }
            KernelRule::KModalDiamond
        }
        CoreTerm::ModalBigAnd(args) => {
            for a in args.iter() {
                if let Some(p) = record_inference(ctx, a, axioms) {
                    premises.push(p);
                }
            }
            KernelRule::KModalBigAnd
        }
        // cohesive modalities ∫ ⊣ ♭ ⊣ ♯.
        CoreTerm::Shape(t) => {
            if let Some(p) = record_inference(ctx, t, axioms) {
                premises.push(p);
            }
            KernelRule::KShape
        }
        CoreTerm::Flat(t) => {
            if let Some(p) = record_inference(ctx, t, axioms) {
                premises.push(p);
            }
            KernelRule::KFlat
        }
        CoreTerm::Sharp(t) => {
            if let Some(p) = record_inference(ctx, t, axioms) {
                premises.push(p);
            }
            KernelRule::KSharp
        }
    };
    Some((rule, premises))
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
        let all_known = names.iter().all(|n| is_known_rule("z3", n.as_str()));
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
        let all_known = names.iter().all(|n| is_known_rule("z3", n.as_str()));
        assert!(
            all_known,
            "every rule in trace should be known: {:?}",
            names.iter().map(|n| n.as_str()).collect::<Vec<_>>()
        );
    }

    // -- Phase 2 replay ----------------------------------------------

    #[test]
    fn replay_asserted_requires_reconstruction() {
        for rule in &["asserted"] {
            let tree = parse_sexpr(&format!("({rule} dummy)")).unwrap();
            assert!(
                matches!(replay_z3_tree(&tree), Err(crate::KernelError::UnsupportedSmtProofTree { backend, rule: found })
                if backend.as_str() == "z3" && found.as_str() == *rule)
            );
        }
    }

    #[test]
    fn replay_refl_symm_trans_mp_all_require_reconstruction() {
        for rule in &["refl", "symm", "trans", "mp", "hypothesis"] {
            let tree = parse_sexpr(&format!("({rule} dummy)")).unwrap();
            assert!(
                matches!(replay_z3_tree(&tree), Err(crate::KernelError::UnsupportedSmtProofTree { backend, rule: found })
                if backend.as_str() == "z3" && found.as_str() == *rule)
            );
        }
    }

    #[test]
    fn replay_recognizes_but_refuses_every_catalogued_rule() {
        for rule in Z3_KNOWN_RULES {
            let tree = parse_sexpr(&format!("({rule} dummy)")).unwrap();
            assert_eq!(
                inspect_proof_tree(&tree, TreeDialect::Z3).unwrap().as_str(),
                *rule
            );
            assert!(
                matches!(replay_z3_tree(&tree), Err(crate::KernelError::UnsupportedSmtProofTree { backend, rule: found })
                if backend.as_str() == "z3" && found.as_str() == *rule)
            );
        }
    }

    #[test]
    fn replay_cluster_2_rewrite_family_all_require_reconstruction() {
        for rule in &[
            "rewrite",
            "commutativity",
            "distributivity",
            "def-axiom",
            "let-elim",
            "der",
        ] {
            let tree = parse_sexpr(&format!("({rule} dummy)")).unwrap();
            assert!(
                matches!(replay_z3_tree(&tree), Err(crate::KernelError::UnsupportedSmtProofTree { backend, rule: found })
                if backend.as_str() == "z3" && found.as_str() == *rule)
            );
        }
    }

    #[test]
    fn replay_cluster_3_quantifier_family_all_require_reconstruction() {
        for rule in &[
            "monotonicity",
            "quant-intro",
            "pull-quant",
            "push-quant",
            "nnf-pos",
            "nnf-neg",
        ] {
            let tree = parse_sexpr(&format!("({rule} dummy)")).unwrap();
            assert!(
                matches!(replay_z3_tree(&tree), Err(crate::KernelError::UnsupportedSmtProofTree { backend, rule: found })
                if backend.as_str() == "z3" && found.as_str() == *rule)
            );
        }
    }

    #[test]
    fn replay_cluster_4_boolean_family_all_require_reconstruction() {
        for rule in &["iff-true", "iff-false", "and-elim", "not-or-elim"] {
            let tree = parse_sexpr(&format!("({rule} dummy)")).unwrap();
            assert!(
                matches!(replay_z3_tree(&tree), Err(crate::KernelError::UnsupportedSmtProofTree { backend, rule: found })
                if backend.as_str() == "z3" && found.as_str() == *rule)
            );
        }
    }

    #[test]
    fn replay_cluster_5_theory_family_all_require_reconstruction() {
        for rule in &[
            "th-lemma",
            "unit-resolution",
            "lemma",
            "goal",
            "modus-ponens",
        ] {
            let tree = parse_sexpr(&format!("({rule} dummy)")).unwrap();
            assert!(
                matches!(replay_z3_tree(&tree), Err(crate::KernelError::UnsupportedSmtProofTree { backend, rule: found })
                if backend.as_str() == "z3" && found.as_str() == *rule)
            );
        }
    }

    #[test]
    fn replay_unknown_rule_rejected_by_allowlist() {
        let tree = parse_sexpr("(fabricate_unsat dummy)").unwrap();
        let err = replay_z3_tree(&tree).unwrap_err();
        assert!(matches!(err, crate::KernelError::UnknownRule { .. }));
    }

    #[test]
    fn replay_rejects_bare_atom() {
        let tree = parse_sexpr("justanatom").unwrap();
        let err = replay_z3_tree(&tree).unwrap_err();
        match err {
            crate::KernelError::SmtReplayFailed { reason } => {
                assert!(reason.as_str().contains("bare atom"));
            }
            other => panic!("expected SmtReplayFailed, got {:?}", other),
        }
    }

    // -- Hierarchical composition ------------------------------------

    #[test]
    fn nested_z3_proof_cannot_compose_unchecked_witnesses() {
        let tree = parse_sexpr("(mp (refl x) (asserted y))").unwrap();
        assert_eq!(
            inspect_proof_tree(&tree, TreeDialect::Z3).unwrap().as_str(),
            "mp"
        );
        assert!(matches!(
            replay_z3_tree(&tree),
            Err(crate::KernelError::UnsupportedSmtProofTree { .. })
        ));
    }

    #[test]
    fn atom_children_are_not_recursively_replayed() {
        for rule in &["refl"] {
            let tree = parse_sexpr(&format!("({rule} dummy)")).unwrap();
            assert!(
                matches!(replay_z3_tree(&tree), Err(crate::KernelError::UnsupportedSmtProofTree { backend, rule: found })
                if backend.as_str() == "z3" && found.as_str() == *rule)
            );
        }
    }

    #[test]
    fn nested_forged_rule_fails_at_any_depth() {
        // A forged rule nested inside a legitimate outer rule
        // must still be caught. This validates that the
        // allowlist check runs at every recursion level.
        let tree = parse_sexpr("(mp (fabricate x) (asserted y))").unwrap();
        let err = replay_z3_tree(&tree).unwrap_err();
        assert!(matches!(err, crate::KernelError::UnknownRule { .. }));
    }

    // -- ALETHE replay ------------------------------------------------

    #[test]
    fn replay_aletha_direct_rule_name_requires_reconstruction() {
        let tree = parse_sexpr("(resolution p1 p2)").unwrap();
        assert_eq!(
            inspect_proof_tree(&tree, TreeDialect::Alethe)
                .unwrap()
                .as_str(),
            "resolution"
        );
        assert!(
            matches!(replay_aletha_tree(&tree), Err(crate::KernelError::UnsupportedSmtProofTree { backend, rule })
            if backend.as_str() == "aletha" && rule.as_str() == "resolution")
        );
    }

    #[test]
    fn replay_aletha_step_node_extracts_rule_keyword() {
        let tree = parse_sexpr("(step t1 :rule refl :premises ())").unwrap();
        assert_eq!(
            inspect_proof_tree(&tree, TreeDialect::Alethe)
                .unwrap()
                .as_str(),
            "refl"
        );
        assert!(
            matches!(replay_aletha_tree(&tree), Err(crate::KernelError::UnsupportedSmtProofTree { backend, rule })
            if backend.as_str() == "aletha" && rule.as_str() == "refl")
        );
    }

    #[test]
    fn replay_aletha_step_without_rule_keyword_rejected() {
        let trace = "(step t1 :premises ())";
        let tree = parse_sexpr(trace).unwrap();
        let err = replay_aletha_tree(&tree).unwrap_err();
        match err {
            crate::KernelError::SmtReplayFailed { reason } => {
                assert!(reason.as_str().contains(":rule"));
            }
            other => panic!("expected SmtReplayFailed, got {:?}", other),
        }
    }

    #[test]
    fn replay_aletha_unknown_rule_rejected_by_allowlist() {
        let tree = parse_sexpr("(fabricate_rule dummy)").unwrap();
        let err = replay_aletha_tree(&tree).unwrap_err();
        assert!(matches!(err, crate::KernelError::UnknownRule { .. }));
    }

    #[test]
    fn replay_aletha_recognizes_but_refuses_catalogued_rules() {
        // Diagnostic recognition must never become proof authority.
        for rule in CVC5_ALETHE_KNOWN_RULES {
            // Skip `step` itself — it's a meta-node wrapper,
            // not a rule. Its body carries a `:rule` keyword
            // that names the real rule.
            if *rule == "step" || *rule == "assume" || *rule == "anchor" {
                continue;
            }
            let tree = parse_sexpr(&format!("({} dummy)", rule)).unwrap();
            assert_eq!(
                inspect_proof_tree(&tree, TreeDialect::Alethe)
                    .unwrap()
                    .as_str(),
                *rule
            );
            assert!(
                matches!(replay_aletha_tree(&tree), Err(crate::KernelError::UnsupportedSmtProofTree { backend, rule: found })
                    if backend.as_str() == "aletha" && found.as_str() == *rule)
            );
        }
    }

    #[test]
    fn replay_aletha_rejects_bare_atom() {
        let tree = parse_sexpr("justanatom").unwrap();
        let err = replay_aletha_tree(&tree).unwrap_err();
        match err {
            crate::KernelError::SmtReplayFailed { reason } => {
                assert!(reason.as_str().contains("bare atom"));
            }
            other => panic!("expected SmtReplayFailed, got {:?}", other),
        }
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

    // -- V-stage + citation wiring --------------------

    #[test]
    fn quotient_rules_report_v8_stage() {
        // Quot-Form / Intro / Elim must report V8.
        // Pre-#240 these silently fell through to "V0" because the
        // v_stage match only listed the original eight V8 rules.
        assert_eq!(KernelRule::KQuotForm.v_stage(), "V8");
        assert_eq!(KernelRule::KQuotIntro.v_stage(), "V8");
        assert_eq!(KernelRule::KQuotElim.v_stage(), "V8");
    }

    #[test]
    fn round_trip_rule_registered_with_abstract_category() {
        // K-Round-Trip is registered with current stage and the
        // abstract semantic category "OC / DC translation
        // round-trip" — no internal-spec ticket leakage.
        assert_eq!(KernelRule::KRoundTrip.name(), "K-Round-Trip");
        assert_eq!(KernelRule::KRoundTrip.v_stage(), "V0");
        assert_eq!(
            KernelRule::KRoundTrip.category(),
            Some("OC / DC translation round-trip")
        );
    }

    #[test]
    fn category_returns_abstract_labels_only() {
        // Smoke check: a representative sample of rules report
        // category labels that name the *kind* of rule, with no
        // internal-spec section / ticket references.
        let cats: Vec<&str> = [
            KernelRule::KQuotForm,
            KernelRule::KShape,
            KernelRule::KPathOverForm,
            KernelRule::KEpsilonOf,
            KernelRule::KRoundTrip,
            KernelRule::KSmt,
            KernelRule::KFwAx,
        ]
        .iter()
        .filter_map(|r| r.category())
        .collect();
        for c in &cats {
            assert!(
                !c.contains("§") && !c.contains("VVA") && !c.contains("#"),
                "category `{}` must be an abstract label without spec refs",
                c
            );
        }
    }

    #[test]
    fn cohesive_rules_report_v8_stage() {
        // Shape / Flat / Sharp must report V8.
        assert_eq!(KernelRule::KShape.v_stage(), "V8");
        assert_eq!(KernelRule::KFlat.v_stage(), "V8");
        assert_eq!(KernelRule::KSharp.v_stage(), "V8");
    }

    #[test]
    fn quotient_rules_in_quotient_types_category() {
        let cat = Some("Quotient types");
        assert_eq!(KernelRule::KQuotForm.category(), cat);
        assert_eq!(KernelRule::KQuotIntro.category(), cat);
        assert_eq!(KernelRule::KQuotElim.category(), cat);
    }

    #[test]
    fn cohesive_rules_in_cohesive_modalities_category() {
        let cat = Some("Cohesive modalities");
        assert_eq!(KernelRule::KShape.category(), cat);
        assert_eq!(KernelRule::KFlat.category(), cat);
        assert_eq!(KernelRule::KSharp.category(), cat);
    }

    #[test]
    fn elim_rule_in_inductive_eliminator_category() {
        assert_eq!(
            KernelRule::KElim.category(),
            Some("Inductive type — eliminator")
        );
    }

    #[test]
    fn variable_rule_has_no_specific_category() {
        // K-Var is universal across every dependent type theory;
        // honest answer is `None` rather than backfilling a category.
        assert_eq!(KernelRule::KVar.category(), None);
    }

    #[test]
    fn category_present_for_every_concrete_rule() {
        // Every named rule (excluding K-Var which is universal)
        // must carry an abstract semantic category so the audit
        // report can group rules by domain.
        let concrete_rules = [
            KernelRule::KQuotForm,
            KernelRule::KQuotIntro,
            KernelRule::KQuotElim,
            KernelRule::KShape,
            KernelRule::KFlat,
            KernelRule::KSharp,
            KernelRule::KElim,
            KernelRule::KRoundTrip,
            KernelRule::KPathOverForm,
        ];
        for rule in concrete_rules {
            assert!(
                rule.category().is_some(),
                "rule {rule:?} must carry an abstract category"
            );
        }
    }

    #[test]
    fn meta_accessors_agree_for_every_rule() {
        // Pin: the three accessor methods return exactly the same
        // values as projecting `meta()`. This makes silent drift
        // structurally impossible — any future "optimisation" that
        // re-implements name/v_stage/category bypassing meta() must
        // either fix all three together or break this test.
        for rule in KernelRule::full_list() {
            let m = rule.meta();
            assert_eq!(rule.name(), m.name, "name() drift on {rule:?}");
            assert_eq!(rule.v_stage(), m.v_stage, "v_stage() drift on {rule:?}");
            assert_eq!(rule.category(), m.category, "category() drift on {rule:?}");
        }
    }

    #[test]
    fn full_list_covers_every_variant() {
        // Pin: every distinct enum variant appears exactly once in
        // `full_list()`. Adding a new variant to `KernelRule` and
        // forgetting to register it in `full_list()` is a load-
        // bearing omission — this test surfaces it as a uniqueness
        // failure, and the per-rule kernel-soundness corpus drift
        // check catches the count mismatch separately.
        let list = KernelRule::full_list();
        let mut seen: std::collections::BTreeSet<&KernelRule> = std::collections::BTreeSet::new();
        // KernelRule derives PartialEq + Eq + Hash but not Ord; use
        // a HashSet-style uniqueness pin via a Vec sweep instead.
        let mut as_vec: Vec<&KernelRule> = list.iter().collect();
        as_vec.sort_by_key(|r| r.name());
        for w in as_vec.windows(2) {
            assert_ne!(
                w[0].name(),
                w[1].name(),
                "duplicate entry in KernelRule::full_list(): {}",
                w[0].name(),
            );
        }
        // Discharge the unused-variable warning on `seen`.
        seen.clear();
    }

    #[test]
    fn full_list_count_matches_soundness_corpus() {
        // Pin: KernelRule::full_list().len() agrees with the
        // EXPECTED_KERNEL_RULE_COUNT anchor in soundness/mod.rs.
        // Drift here means a new rule was added to the enum + the
        // full_list array, but the per-foundation soundness corpus
        // wasn't updated — surfaces as an export-side test failure
        // separately, but pinning at the enum level catches it
        // earlier.
        assert_eq!(
            KernelRule::full_list().len(),
            crate::soundness::EXPECTED_KERNEL_RULE_COUNT,
            "KernelRule::full_list() ({} entries) does not agree with \
             EXPECTED_KERNEL_RULE_COUNT ({})",
            KernelRule::full_list().len(),
            crate::soundness::EXPECTED_KERNEL_RULE_COUNT,
        );
    }
}
