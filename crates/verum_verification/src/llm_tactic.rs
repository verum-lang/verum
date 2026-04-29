//! LLM-native tactic protocol — LCF-style fail-closed bridge between
//! a language-model proof proposer and the trusted kernel.
//!
//! ## Goal
//!
//! Verum is the first proof assistant where LLM assistance is
//! guaranteed sound *by construction*.  An LLM may propose tactic
//! sequences for any goal, but the proposal is **always re-checked
//! by the kernel** before being committed.  If the kernel rejects
//! any step, the proposal is discarded and the audit trail records
//! the rejection.  The LLM never short-circuits the kernel.
//!
//! This is the LCF principle, generalised: every term is kernel-
//! checked regardless of who / what proposed it.
//!
//! ## Architectural pattern
//!
//! Same single-trait-boundary pattern as the rest of the integration
//! arc (ladder_dispatch / proof_drafting / proof_repair / closure_cache
//! / doc_render / foreign_import):
//!
//!   * [`LlmGoalSummary`] — typed projection of the focused proof
//!     state (goal + hypotheses + lemmas + recent history + framework
//!     axioms in scope) handed to the LLM.
//!   * [`LlmProofProposal`] — typed result (model_id + prompt_hash +
//!     completion_hash + tactic_sequence + raw_completion).
//!   * [`LlmTacticAdapter`] trait — single dispatch interface.
//!     Reference impls: [`MockLlmAdapter`] (deterministic,
//!     test-friendly), [`EchoLlmAdapter`] (echoes a configured
//!     hint).  Production adapters (cloud / local) plug in via the
//!     same trait.
//!   * [`KernelChecker`] trait — re-checks each proposed step.
//!     Reference impl: [`PatternKernelChecker`] (V0 — recognises
//!     well-formed `apply NAME` / canonical-tactic shapes).  V1
//!     wires the actual kernel re-check.
//!   * [`KernelGate`] — orchestrates `adapter.propose` →
//!     `checker.check_step` per-step → typed [`KernelVerdict`] +
//!     [`LlmProtocolEvent`] for the audit trail.
//!   * [`AuditTrail`] — append-only JSONL log persisted to
//!     `target/.verum_cache/llm-proofs.jsonl` (or wherever the
//!     consumer points it).  Every LLM invocation produces an
//!     event so the proof is reproducible from the log.
//!
//! ## Fail-closed contract
//!
//! `KernelGate::run` returns [`KernelVerdict::Accepted`] only when
//! the kernel re-checked every step in the proposal.  Any rejection
//! produces [`KernelVerdict::Rejected`] with the failing step's
//! index + reason.  The audit trail captures both paths.

use serde::{Deserialize, Serialize};
use std::path::Path;
use verum_common::Text;

// =============================================================================
// LlmGoalSummary — typed projection of the proof state
// =============================================================================

/// What the LLM sees about the focused proof state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmGoalSummary {
    pub theorem_name: Text,
    pub focused_proposition: Text,
    /// Local hypotheses: `(name, type)`.
    pub hypotheses: Vec<(Text, Text)>,
    /// Lemmas reachable from this proof: `(name, signature)`.
    pub lemmas_in_scope: Vec<(Text, Text)>,
    /// Last few executed tactic steps (the LLM uses this for
    /// context — repeating the same step twice is usually wrong).
    pub recent_tactic_history: Vec<Text>,
    /// Framework axioms reachable: `(framework_tag, citation)`.
    /// The LLM uses these to know which trusted-base citations are
    /// legitimate.
    pub framework_axioms_in_scope: Vec<(Text, Text)>,
}

impl LlmGoalSummary {
    /// Convenient constructor for tests.
    pub fn new(theorem_name: impl Into<Text>, proposition: impl Into<Text>) -> Self {
        Self {
            theorem_name: theorem_name.into(),
            focused_proposition: proposition.into(),
            hypotheses: Vec::new(),
            lemmas_in_scope: Vec::new(),
            recent_tactic_history: Vec::new(),
            framework_axioms_in_scope: Vec::new(),
        }
    }

    /// Render the goal into the canonical prompt the adapters
    /// hash and feed to the LLM.  Order is stable so the prompt
    /// hash is deterministic across runs.
    pub fn render_prompt(&self) -> Text {
        let mut s = String::new();
        s.push_str("You are a proof assistant. Propose a Verum tactic sequence to discharge the goal.\n\n");
        s.push_str(&format!("Theorem: {}\n", self.theorem_name.as_str()));
        s.push_str(&format!("Goal: {}\n", self.focused_proposition.as_str()));
        if !self.hypotheses.is_empty() {
            s.push_str("\nHypotheses:\n");
            for (name, ty) in &self.hypotheses {
                s.push_str(&format!("  {} : {}\n", name.as_str(), ty.as_str()));
            }
        }
        if !self.lemmas_in_scope.is_empty() {
            s.push_str("\nLemmas in scope:\n");
            for (name, sig) in &self.lemmas_in_scope {
                s.push_str(&format!("  {} : {}\n", name.as_str(), sig.as_str()));
            }
        }
        if !self.recent_tactic_history.is_empty() {
            s.push_str("\nRecent tactic history (most recent last):\n");
            for step in &self.recent_tactic_history {
                s.push_str(&format!("  - {}\n", step.as_str()));
            }
        }
        if !self.framework_axioms_in_scope.is_empty() {
            s.push_str("\nFramework axioms in scope:\n");
            for (tag, cite) in &self.framework_axioms_in_scope {
                s.push_str(&format!("  {}: {}\n", tag.as_str(), cite.as_str()));
            }
        }
        s.push_str("\nRespond with a sequence of tactic invocations, one per line.\n");
        Text::from(s)
    }

    /// Stable blake3 hash of the rendered prompt.  Hex-encoded.
    pub fn prompt_hash(&self) -> Text {
        let p = self.render_prompt();
        Text::from(hex32(blake3::hash(p.as_str().as_bytes()).as_bytes()))
    }
}

fn hex32(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// =============================================================================
// LlmProofProposal — what the LLM returned
// =============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmProofProposal {
    pub model_id: Text,
    pub prompt_hash: Text,
    pub completion_hash: Text,
    /// Parsed tactic sequence (one tactic invocation per element).
    pub tactic_sequence: Vec<Text>,
    /// Raw completion as the LLM emitted it (for the audit log).
    pub raw_completion: Text,
    /// Wall time for the propose call (ms).
    pub elapsed_ms: u64,
}

impl LlmProofProposal {
    /// Build the completion-side hash from a raw string.
    pub fn hash_completion(s: &str) -> Text {
        Text::from(hex32(blake3::hash(s.as_bytes()).as_bytes()))
    }
}

// =============================================================================
// LlmTacticAdapter trait
// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum LlmError {
    /// The model service is unreachable / failed.
    Transport(Text),
    /// The model returned a malformed response.
    MalformedResponse(Text),
    /// The model declined to answer (e.g. content policy).
    Refused(Text),
    /// Configuration error (e.g. unknown model id).
    Config(Text),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Transport(t) => write!(f, "transport: {}", t.as_str()),
            LlmError::MalformedResponse(t) => write!(f, "malformed response: {}", t.as_str()),
            LlmError::Refused(t) => write!(f, "model refused: {}", t.as_str()),
            LlmError::Config(t) => write!(f, "config: {}", t.as_str()),
        }
    }
}

impl std::error::Error for LlmError {}

/// Single dispatch interface for an LLM proof-proposer.
pub trait LlmTacticAdapter: std::fmt::Debug + Send + Sync {
    /// Stable model identifier (e.g. `"local/llama-3-8b-q4"`,
    /// `"cloud/claude-sonnet-4-6"`).  Goes into the audit trail.
    fn model_id(&self) -> Text;

    /// Propose a tactic sequence for the given goal.  The
    /// implementation MUST be deterministic on the same `(prompt,
    /// model_id)` pair *or* explicitly mark itself as
    /// non-deterministic via an extra-info channel — the audit
    /// trail relies on this for replay.
    fn propose(&self, goal: &LlmGoalSummary) -> Result<LlmProofProposal, LlmError>;
}

// =============================================================================
// MockLlmAdapter — deterministic, test-friendly
// =============================================================================

/// Mock adapter that returns a canned proposal for every prompt.
/// Used in tests + the CLI's `--mock` mode.
#[derive(Debug, Clone)]
pub struct MockLlmAdapter {
    pub model_id: Text,
    /// Canned tactic sequence to return.
    pub canned_steps: Vec<Text>,
}

impl MockLlmAdapter {
    pub fn new(model_id: impl Into<Text>, steps: Vec<&str>) -> Self {
        Self {
            model_id: model_id.into(),
            canned_steps: steps.iter().map(|s| Text::from(*s)).collect(),
        }
    }
}

impl LlmTacticAdapter for MockLlmAdapter {
    fn model_id(&self) -> Text {
        self.model_id.clone()
    }

    fn propose(&self, goal: &LlmGoalSummary) -> Result<LlmProofProposal, LlmError> {
        let raw = self
            .canned_steps
            .iter()
            .map(|t| t.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(LlmProofProposal {
            model_id: self.model_id.clone(),
            prompt_hash: goal.prompt_hash(),
            completion_hash: LlmProofProposal::hash_completion(&raw),
            tactic_sequence: self.canned_steps.clone(),
            raw_completion: Text::from(raw),
            elapsed_ms: 0,
        })
    }
}

// =============================================================================
// EchoLlmAdapter — echoes a hint as the proposal (useful for piping
// in pre-computed sequences from external tools)
// =============================================================================

#[derive(Debug, Clone)]
pub struct EchoLlmAdapter {
    pub model_id: Text,
    pub hint_lines: Vec<Text>,
}

impl EchoLlmAdapter {
    pub fn new(model_id: impl Into<Text>, hint: &str) -> Self {
        let hint_lines: Vec<Text> = hint
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(Text::from)
            .collect();
        Self {
            model_id: model_id.into(),
            hint_lines,
        }
    }
}

impl LlmTacticAdapter for EchoLlmAdapter {
    fn model_id(&self) -> Text {
        self.model_id.clone()
    }

    fn propose(&self, goal: &LlmGoalSummary) -> Result<LlmProofProposal, LlmError> {
        if self.hint_lines.is_empty() {
            return Err(LlmError::MalformedResponse(Text::from(
                "echo adapter has no hint lines configured",
            )));
        }
        let raw = self
            .hint_lines
            .iter()
            .map(|t| t.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(LlmProofProposal {
            model_id: self.model_id.clone(),
            prompt_hash: goal.prompt_hash(),
            completion_hash: LlmProofProposal::hash_completion(&raw),
            tactic_sequence: self.hint_lines.clone(),
            raw_completion: Text::from(raw),
            elapsed_ms: 0,
        })
    }
}

// =============================================================================
// KernelChecker trait
// =============================================================================

/// Typed rejection reason from a kernel re-check.  Replaces the
/// stringly-typed Text-only rejection so callers (LLM auditing,
/// CLI metrics, replay engines) can branch on the failure class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelRejectReason {
    /// Step references a name not in the goal's textual lemma list
    /// AND not in the kernel's axiom registry.
    NotInScope { name: Text },
    /// Name resolves textually but isn't a registered kernel
    /// axiom.  Production mode rejects these as "unattested" —
    /// the LLM would otherwise be free to invent lemma names.
    NotKernelAttested { name: Text },
    /// The tactic head isn't recognised (neither `apply NAME` nor
    /// a canonical tactic word).
    UnknownTactic { head: Text },
    /// Tactic application is malformed (missing argument,
    /// unparenthesised, …).
    MalformedSyntax { detail: Text },
    /// Catch-all for back-compat with `Result<(), Text>`-shaped
    /// implementations.
    Other(Text),
}

impl KernelRejectReason {
    /// Render as a single-line diagnostic.  Used by the back-compat
    /// `check_step` shim that returns `Result<(), Text>`.
    pub fn render(&self) -> Text {
        match self {
            Self::NotInScope { name } => Text::from(format!(
                "lemma '{}' not in scope (apply target unresolved)",
                name.as_str()
            )),
            Self::NotKernelAttested { name } => Text::from(format!(
                "lemma '{}' resolves textually but is not a registered kernel axiom — \
                 production mode requires kernel attestation",
                name.as_str()
            )),
            Self::UnknownTactic { head } => Text::from(format!(
                "unrecognised tactic shape '{}' — accept only `apply NAME` or canonical tactics",
                head.as_str()
            )),
            Self::MalformedSyntax { detail } => Text::from(format!(
                "malformed tactic: {}",
                detail.as_str()
            )),
            Self::Other(t) => t.clone(),
        }
    }

    /// Stable kebab-case label for telemetry.
    pub fn label(&self) -> &'static str {
        match self {
            Self::NotInScope { .. } => "not-in-scope",
            Self::NotKernelAttested { .. } => "not-kernel-attested",
            Self::UnknownTactic { .. } => "unknown-tactic",
            Self::MalformedSyntax { .. } => "malformed-syntax",
            Self::Other(_) => "other",
        }
    }
}

/// Re-checks one proposed tactic step against the goal.
///
/// Two implementations ship:
///
///   * [`PatternKernelChecker`] — text-shape recogniser; accepts
///     `apply NAME` when `NAME` is in the goal's textual lemma
///     list, plus a fixed canonical-tactic head set.  V0 mode.
///   * [`KernelInferChecker`] — production hardening (#90).
///     Carries a `verum_kernel::AxiomRegistry`; `apply NAME` MUST
///     resolve through the registry (kernel-attested), not just
///     through the LLM's textual context.
///
/// Both honour the **fail-closed contract**: if the checker can't
/// *prove* the step is sound it MUST reject.
pub trait KernelChecker: std::fmt::Debug + Send + Sync {
    /// Primary check.  Returns Ok on accept; Err with a stringly-
    /// typed diagnostic on reject.  Implementors that produce
    /// structured rejections should override [`check_step_typed`]
    /// instead and project the Text via `KernelRejectReason::render`.
    fn check_step(&self, goal: &LlmGoalSummary, step: &str) -> Result<(), Text>;

    /// Typed entry point.  Default impl wraps `check_step`'s Text
    /// in [`KernelRejectReason::Other`]; impls that want structured
    /// reasons override this method (and make `check_step` call it
    /// + `.render()`).
    fn check_step_typed(
        &self,
        goal: &LlmGoalSummary,
        step: &str,
    ) -> Result<(), KernelRejectReason> {
        self.check_step(goal, step).map_err(KernelRejectReason::Other)
    }
}

/// V0 reference checker.  Recognises:
///   * `apply <name>` where `<name>` is one of the lemmas in scope.
///   * Canonical tactic invocations: `intro`, `intro <name>`,
///     `auto`, `simp`, `refl`, `assumption`, `trivial`, `ring`,
///     `linarith`, `nlinarith`, `norm_num`, `omega`, `field`,
///     `blast`, `smt`.
///   * Lines starting with `//` (comments) — admitted as no-ops.
///
/// Anything else is rejected.  V1 wires in the full kernel
/// re-check (refinement-type elaboration, depth check, framework
/// axiom resolution, …).
#[derive(Debug, Default, Clone, Copy)]
pub struct PatternKernelChecker;

impl PatternKernelChecker {
    pub fn new() -> Self {
        Self
    }
}

/// Public accessor for the canonical-tactic set.  Used by sibling
/// modules (e.g. `proof_repl`'s GoalRewriter surface-alignment
/// invariant) to ensure their dispatch surface stays in sync.
pub fn canonical_tactics() -> &'static [&'static str] {
    CANONICAL_TACTICS
}

/// Canonical tactic heads accepted by `parse_step`.  Every entry is
/// either:
///
///   * A name from `verum_verification::tactic_combinator::TacticCombinator`
///     (`skip` / `fail` / etc.); or
///   * A canonical decision-procedure / surface tactic the catalogue
///     documents elsewhere (`auto` / `simp` / `linarith` / etc.).
///
/// Adding a new head here is the right place to extend the
/// PatternKernelChecker's accept set.  The KernelInferChecker layers
/// kernel-attestation on top of this for `apply NAME` resolution.
const CANONICAL_TACTICS: &[&str] = &[
    // ----- core combinator surface (matches TacticCombinator::all) -----
    "skip",
    "fail",
    // ----- proof-state navigation -----
    "intro",
    "intros",
    "revert",
    "case",
    "cases",
    "destruct",
    "induction",
    // ----- soundness-trivial closers -----
    "refl",
    "reflexivity",
    "trivial",
    "assumption",
    "exact",
    // ----- contradiction family -----
    "contradiction",
    "by_contradiction",
    "exfalso",
    // ----- conjunction / disjunction / quantifier introduction -----
    "split",
    "left",
    "right",
    "constructor",
    "exists",
    // ----- decision procedures -----
    "auto",
    "eauto",
    "blast",
    "smt",
    "decide",
    "tauto",
    // ----- arithmetic decision procedures -----
    "ring",
    "field",
    "linarith",
    "nlinarith",
    "lia",
    "nlia",
    "lra",
    "nra",
    "omega",
    "norm_num",
    // ----- equality manipulation -----
    "rewrite",
    "rw",
    "subst",
    "unfold",
    "fold",
    "simp",
    "simplify",
    "compute",
    "congruence",
];

/// Common parse: project `step` to a typed shape that both
/// `PatternKernelChecker` and `KernelInferChecker` consume.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedStep<'a> {
    /// Empty / comment line — admit unconditionally.
    Admitted,
    /// `apply NAME [...]` — name extracted, fully verified by caller.
    Apply { name: &'a str },
    /// Canonical tactic head (`auto`, `simp`, …).
    Canonical { head: &'a str },
    /// Anything else; carries the head for diagnostics.
    Unknown { head: String },
    /// `apply` with no name argument.
    ApplyMissingName,
}

fn parse_step(step: &str) -> ParsedStep<'_> {
    let s = step.trim().trim_end_matches(';').trim();
    if s.is_empty() || s.starts_with("//") {
        return ParsedStep::Admitted;
    }
    if let Some(rest) = s.strip_prefix("apply ") {
        let name = rest.split_whitespace().next().unwrap_or("");
        let bare = name.trim_end_matches(',');
        if bare.is_empty() {
            return ParsedStep::ApplyMissingName;
        }
        return ParsedStep::Apply { name: bare };
    }
    let head = s
        .split(|c: char| !c.is_ascii_alphabetic() && c != '_')
        .next()
        .unwrap_or("");
    if CANONICAL_TACTICS.contains(&head) {
        return ParsedStep::Canonical { head };
    }
    ParsedStep::Unknown {
        head: head.to_string(),
    }
}

impl KernelChecker for PatternKernelChecker {
    fn check_step(&self, goal: &LlmGoalSummary, step: &str) -> Result<(), Text> {
        self.check_step_typed(goal, step).map_err(|r| r.render())
    }

    fn check_step_typed(
        &self,
        goal: &LlmGoalSummary,
        step: &str,
    ) -> Result<(), KernelRejectReason> {
        match parse_step(step) {
            ParsedStep::Admitted | ParsedStep::Canonical { .. } => Ok(()),
            ParsedStep::ApplyMissingName => Err(KernelRejectReason::MalformedSyntax {
                detail: Text::from("`apply` with no lemma name"),
            }),
            ParsedStep::Apply { name } => {
                let resolved = goal
                    .lemmas_in_scope
                    .iter()
                    .any(|(n, _)| n.as_str() == name);
                if resolved {
                    Ok(())
                } else {
                    Err(KernelRejectReason::NotInScope {
                        name: Text::from(name),
                    })
                }
            }
            ParsedStep::Unknown { head } => Err(KernelRejectReason::UnknownTactic {
                head: Text::from(head),
            }),
        }
    }
}

// =============================================================================
// KernelInferChecker — production: kernel-attested apply-resolution (#90)
// =============================================================================
//
// Pre-this-module `PatternKernelChecker` accepted `apply NAME` as long
// as `NAME` appeared in the goal's textual `lemmas_in_scope` list —
// which is whatever the LLM-prompt-builder rendered into the goal
// view.  An adversarial LLM could exploit that by constructing a
// goal with a fictitious lemma in scope.
//
// Hardening: the production checker carries a
// `verum_kernel::AxiomRegistry` — the *kernel-side* trust boundary
// — and resolves `apply NAME` through it.  A name that's only in
// the textual lemma list but not registered as an axiom or
// definition is rejected as `NotKernelAttested`.
//
// This closes the LCF gate at the layer the `KernelGate` orchestrator
// asked for: every accepted `apply` step is provably a citation of
// a registered kernel axiom — solver-side proof reconstruction is
// no longer in the trust path for citation-resolution.

/// Production kernel re-checker.  Resolves `apply NAME` through a
/// kernel `AxiomRegistry` (rather than through the LLM's textual
/// goal context, which is untrusted).
#[derive(Debug, Clone)]
pub struct KernelInferChecker {
    registry: verum_kernel::AxiomRegistry,
}

impl KernelInferChecker {
    /// Build a checker carrying the running kernel's
    /// `AxiomRegistry`.  Callers (CLI, REPL, batch) thread their
    /// production registry in here.
    pub fn new(registry: verum_kernel::AxiomRegistry) -> Self {
        Self { registry }
    }

    /// Accessor — useful for diagnostics and for the `KernelGate`
    /// to introspect which axioms a session has admitted.
    pub fn registry(&self) -> &verum_kernel::AxiomRegistry {
        &self.registry
    }

    /// Lookup a lemma name in the kernel registry.  Returns true
    /// iff a registered axiom or definition matches by name.
    fn registry_has(&self, name: &str) -> bool {
        self.registry
            .all()
            .iter()
            .any(|a| a.name.as_str() == name)
    }
}

impl KernelChecker for KernelInferChecker {
    fn check_step(&self, goal: &LlmGoalSummary, step: &str) -> Result<(), Text> {
        self.check_step_typed(goal, step).map_err(|r| r.render())
    }

    fn check_step_typed(
        &self,
        goal: &LlmGoalSummary,
        step: &str,
    ) -> Result<(), KernelRejectReason> {
        match parse_step(step) {
            ParsedStep::Admitted | ParsedStep::Canonical { .. } => Ok(()),
            ParsedStep::ApplyMissingName => Err(KernelRejectReason::MalformedSyntax {
                detail: Text::from("`apply` with no lemma name"),
            }),
            ParsedStep::Apply { name } => {
                // Kernel-side resolution: the registry is the
                // authoritative trust boundary for citation
                // attestation.  The textual `lemmas_in_scope` view
                // is *advisory* — useful for diagnostics, never
                // sufficient on its own.
                if self.registry_has(name) {
                    return Ok(());
                }
                // Distinguish the two failure modes so callers can
                // tell "the LLM cited an inscrutable name" from
                // "the LLM cited a name the LLM was told about but
                // the kernel hasn't registered".
                let in_textual_scope = goal
                    .lemmas_in_scope
                    .iter()
                    .any(|(n, _)| n.as_str() == name);
                if in_textual_scope {
                    Err(KernelRejectReason::NotKernelAttested {
                        name: Text::from(name),
                    })
                } else {
                    Err(KernelRejectReason::NotInScope {
                        name: Text::from(name),
                    })
                }
            }
            ParsedStep::Unknown { head } => Err(KernelRejectReason::UnknownTactic {
                head: Text::from(head),
            }),
        }
    }
}

// =============================================================================
// KernelVerdict + LlmProtocolEvent
// =============================================================================

/// Result of routing one LLM proposal through the kernel gate.
#[derive(Debug, Clone, PartialEq)]
pub enum KernelVerdict {
    /// Every step type-checked.
    Accepted { steps_checked: usize },
    /// Some step failed; carries the index (0-based) + reason.
    Rejected {
        failed_step_index: usize,
        reason: Text,
    },
}

impl KernelVerdict {
    pub fn is_accepted(&self) -> bool {
        matches!(self, KernelVerdict::Accepted { .. })
    }
}

/// One audit-trail event.  Persisted as a single JSONL line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum LlmProtocolEvent {
    /// The LLM was queried (regardless of outcome).
    LlmInvoked {
        model_id: Text,
        theorem: Text,
        prompt_hash: Text,
        completion_hash: Text,
        tactic_count: usize,
        elapsed_ms: u64,
        timestamp: u64,
    },
    /// The kernel accepted the proposal.
    KernelAccepted {
        model_id: Text,
        theorem: Text,
        prompt_hash: Text,
        completion_hash: Text,
        steps_checked: usize,
        timestamp: u64,
    },
    /// The kernel rejected at least one step.
    KernelRejected {
        model_id: Text,
        theorem: Text,
        prompt_hash: Text,
        completion_hash: Text,
        failed_step_index: usize,
        reason: Text,
        timestamp: u64,
    },
    /// The adapter itself errored (transport / config / refusal).
    ProtocolError {
        model_id: Text,
        theorem: Text,
        error: Text,
        timestamp: u64,
    },
}

impl LlmProtocolEvent {
    /// Stable diagnostic name (matches the JSON `kind` tag).
    pub fn name(&self) -> &'static str {
        match self {
            Self::LlmInvoked { .. } => "LlmInvoked",
            Self::KernelAccepted { .. } => "KernelAccepted",
            Self::KernelRejected { .. } => "KernelRejected",
            Self::ProtocolError { .. } => "ProtocolError",
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// =============================================================================
// AuditTrail — append-only JSONL log
// =============================================================================

/// Append-only event log.  Implementations may persist to disk
/// (`FilesystemAuditTrail`), an in-memory buffer
/// (`MemoryAuditTrail`, used in tests), or a remote sink.
pub trait AuditTrail: std::fmt::Debug + Send + Sync {
    fn append(&self, event: LlmProtocolEvent) -> Result<(), Text>;
    /// Read every recorded event in chronological order.
    fn read_all(&self) -> Result<Vec<LlmProtocolEvent>, Text>;
}

#[derive(Debug, Default)]
pub struct MemoryAuditTrail {
    events: std::sync::Mutex<Vec<LlmProtocolEvent>>,
}

impl MemoryAuditTrail {
    pub fn new() -> Self {
        Self::default()
    }
}

impl AuditTrail for MemoryAuditTrail {
    fn append(&self, event: LlmProtocolEvent) -> Result<(), Text> {
        let mut g = self
            .events
            .lock()
            .map_err(|_| Text::from("memory audit trail mutex poisoned"))?;
        g.push(event);
        Ok(())
    }

    fn read_all(&self) -> Result<Vec<LlmProtocolEvent>, Text> {
        let g = self
            .events
            .lock()
            .map_err(|_| Text::from("memory audit trail mutex poisoned"))?;
        Ok(g.clone())
    }
}

#[derive(Debug)]
pub struct FilesystemAuditTrail {
    path: std::path::PathBuf,
}

impl FilesystemAuditTrail {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Result<Self, Text> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Text::from(format!("creating audit trail dir {}: {}", parent.display(), e))
            })?;
        }
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl AuditTrail for FilesystemAuditTrail {
    fn append(&self, event: LlmProtocolEvent) -> Result<(), Text> {
        use std::fs::OpenOptions;
        use std::io::Write;
        let json = serde_json::to_string(&event).map_err(|e| {
            Text::from(format!("audit trail serialise: {}", e))
        })?;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| Text::from(format!("audit trail open: {}", e)))?;
        writeln!(f, "{}", json).map_err(|e| Text::from(format!("audit trail write: {}", e)))?;
        Ok(())
    }

    fn read_all(&self) -> Result<Vec<LlmProtocolEvent>, Text> {
        let raw = match std::fs::read_to_string(&self.path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Text::from(format!("audit trail read: {}", e))),
        };
        let mut out = Vec::new();
        for (i, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let event: LlmProtocolEvent = serde_json::from_str(line).map_err(|e| {
                Text::from(format!(
                    "audit trail parse error at line {}: {}",
                    i + 1,
                    e
                ))
            })?;
            out.push(event);
        }
        Ok(out)
    }
}

// =============================================================================
// KernelGate — the LCF-style fail-closed orchestrator
// =============================================================================

/// Orchestrates `adapter.propose` → `checker.check_step` per step →
/// emits the appropriate audit-trail events.
#[derive(Debug, Default, Clone, Copy)]
pub struct KernelGate;

impl KernelGate {
    pub fn new() -> Self {
        Self
    }

    /// Run one round of the protocol.  Returns the verdict and
    /// emits exactly two audit events on the success / failure
    /// paths (LlmInvoked + KernelAccepted, or LlmInvoked +
    /// KernelRejected), or one audit event on transport / config
    /// failure (ProtocolError).
    pub fn run<A: LlmTacticAdapter, K: KernelChecker, T: AuditTrail>(
        &self,
        adapter: &A,
        checker: &K,
        goal: &LlmGoalSummary,
        audit: &T,
    ) -> Result<KernelVerdict, LlmError> {
        let proposal = match adapter.propose(goal) {
            Ok(p) => p,
            Err(e) => {
                // Adapter-side failure — emit ProtocolError and
                // surface the error to the caller.
                let _ = audit.append(LlmProtocolEvent::ProtocolError {
                    model_id: adapter.model_id(),
                    theorem: goal.theorem_name.clone(),
                    error: Text::from(format!("{}", e)),
                    timestamp: now_secs(),
                });
                return Err(e);
            }
        };
        let _ = audit.append(LlmProtocolEvent::LlmInvoked {
            model_id: proposal.model_id.clone(),
            theorem: goal.theorem_name.clone(),
            prompt_hash: proposal.prompt_hash.clone(),
            completion_hash: proposal.completion_hash.clone(),
            tactic_count: proposal.tactic_sequence.len(),
            elapsed_ms: proposal.elapsed_ms,
            timestamp: now_secs(),
        });
        for (i, step) in proposal.tactic_sequence.iter().enumerate() {
            if let Err(reason) = checker.check_step(goal, step.as_str()) {
                let _ = audit.append(LlmProtocolEvent::KernelRejected {
                    model_id: proposal.model_id.clone(),
                    theorem: goal.theorem_name.clone(),
                    prompt_hash: proposal.prompt_hash.clone(),
                    completion_hash: proposal.completion_hash.clone(),
                    failed_step_index: i,
                    reason: reason.clone(),
                    timestamp: now_secs(),
                });
                return Ok(KernelVerdict::Rejected {
                    failed_step_index: i,
                    reason,
                });
            }
        }
        let _ = audit.append(LlmProtocolEvent::KernelAccepted {
            model_id: proposal.model_id.clone(),
            theorem: goal.theorem_name.clone(),
            prompt_hash: proposal.prompt_hash.clone(),
            completion_hash: proposal.completion_hash.clone(),
            steps_checked: proposal.tactic_sequence.len(),
            timestamp: now_secs(),
        });
        Ok(KernelVerdict::Accepted {
            steps_checked: proposal.tactic_sequence.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lemma(name: &str, sig: &str) -> (Text, Text) {
        (Text::from(name), Text::from(sig))
    }

    fn goal_with_lemmas(lemmas: &[(&str, &str)]) -> LlmGoalSummary {
        let mut g = LlmGoalSummary::new("thm", "P(x)");
        g.lemmas_in_scope = lemmas
            .iter()
            .map(|(n, s)| lemma(n, s))
            .collect();
        g
    }

    // ----- LlmGoalSummary -----

    #[test]
    fn render_prompt_is_deterministic() {
        let g1 = LlmGoalSummary::new("foo", "True");
        let g2 = LlmGoalSummary::new("foo", "True");
        assert_eq!(g1.render_prompt(), g2.render_prompt());
        assert_eq!(g1.prompt_hash(), g2.prompt_hash());
    }

    #[test]
    fn prompt_hash_is_64_chars() {
        let g = LlmGoalSummary::new("foo", "P(x)");
        let h = g.prompt_hash();
        assert_eq!(h.as_str().len(), 64);
        assert!(h.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn render_prompt_includes_hypotheses_and_lemmas() {
        let mut g = LlmGoalSummary::new("foo", "Q(x)");
        g.hypotheses = vec![lemma("h1", "P(x)")];
        g.lemmas_in_scope = vec![lemma("p_implies_q", "forall x. P(x) -> Q(x)")];
        let p = g.render_prompt();
        assert!(p.as_str().contains("h1 : P(x)"));
        assert!(p.as_str().contains("p_implies_q"));
    }

    // ----- LlmTacticAdapter impls -----

    #[test]
    fn mock_adapter_returns_canned_steps() {
        let a = MockLlmAdapter::new("mock-1", vec!["intro", "apply foo"]);
        let g = LlmGoalSummary::new("thm", "P");
        let p = a.propose(&g).unwrap();
        assert_eq!(p.model_id.as_str(), "mock-1");
        assert_eq!(p.tactic_sequence.len(), 2);
        assert_eq!(p.tactic_sequence[0].as_str(), "intro");
    }

    #[test]
    fn mock_adapter_hashes_prompt_and_completion() {
        let a = MockLlmAdapter::new("mock", vec!["intro"]);
        let g = LlmGoalSummary::new("thm", "P");
        let p = a.propose(&g).unwrap();
        assert_eq!(p.prompt_hash, g.prompt_hash());
        assert_eq!(p.completion_hash.as_str().len(), 64);
    }

    #[test]
    fn echo_adapter_parses_hint_lines() {
        let a = EchoLlmAdapter::new("echo", "intro\napply foo\n   \nauto");
        let g = LlmGoalSummary::new("thm", "P");
        let p = a.propose(&g).unwrap();
        assert_eq!(p.tactic_sequence.len(), 3);
    }

    #[test]
    fn echo_adapter_rejects_empty_hint() {
        let a = EchoLlmAdapter::new("echo", "");
        let g = LlmGoalSummary::new("thm", "P");
        assert!(matches!(a.propose(&g), Err(LlmError::MalformedResponse(_))));
    }

    // ----- PatternKernelChecker -----

    #[test]
    fn pattern_checker_accepts_canonical_tactics() {
        let c = PatternKernelChecker::new();
        let g = LlmGoalSummary::new("thm", "P");
        for t in &["intro", "auto", "simp", "ring", "linarith", "trivial"] {
            assert!(c.check_step(&g, t).is_ok(), "tactic {} should be accepted", t);
        }
    }

    #[test]
    fn pattern_checker_accepts_apply_with_in_scope_lemma() {
        let c = PatternKernelChecker::new();
        let g = goal_with_lemmas(&[("succ_pos", "...")]);
        assert!(c.check_step(&g, "apply succ_pos").is_ok());
    }

    #[test]
    fn pattern_checker_rejects_apply_with_out_of_scope_lemma() {
        let c = PatternKernelChecker::new();
        let g = goal_with_lemmas(&[("foo", "...")]);
        let err = c.check_step(&g, "apply nonexistent").unwrap_err();
        assert!(err.as_str().contains("not in scope"));
    }

    #[test]
    fn pattern_checker_rejects_garbage() {
        let c = PatternKernelChecker::new();
        let g = LlmGoalSummary::new("thm", "P");
        assert!(c.check_step(&g, "xyz_garbage").is_err());
        assert!(c.check_step(&g, "totally invalid syntax").is_err());
    }

    #[test]
    fn pattern_checker_admits_comments_and_empty_lines() {
        let c = PatternKernelChecker::new();
        let g = LlmGoalSummary::new("thm", "P");
        assert!(c.check_step(&g, "// this is a comment").is_ok());
        assert!(c.check_step(&g, "   ").is_ok());
        assert!(c.check_step(&g, "").is_ok());
    }

    #[test]
    fn pattern_checker_accepts_extended_canonical_tactics() {
        // #105 hardening: every entry in CANONICAL_TACTICS must be
        // admitted as a bare invocation.  The PatternKernelChecker is
        // a pure-pattern recogniser; the kernel-attestation gate
        // KernelInferChecker layers on top.
        let c = PatternKernelChecker::new();
        let g = LlmGoalSummary::new("thm", "P");
        for tac in CANONICAL_TACTICS {
            assert!(
                c.check_step(&g, tac).is_ok(),
                "canonical tactic `{}` should be admitted",
                tac
            );
        }
    }

    #[test]
    fn pattern_checker_accepts_extended_with_argument() {
        // Tactics that take an argument: `cases h`, `induction n`,
        // `unfold foo`, `subst x`.  The pattern checker only inspects
        // the head — argument parsing is the consumer's job.
        let c = PatternKernelChecker::new();
        let g = LlmGoalSummary::new("thm", "P");
        for tac in [
            "cases h", "induction n", "unfold foo_def", "subst x",
            "rewrite h", "rw eq", "exists witness", "constructor",
            "exfalso", "contradiction",
        ] {
            assert!(
                c.check_step(&g, tac).is_ok(),
                "tactic `{}` should be admitted",
                tac
            );
        }
    }

    #[test]
    fn task_105_canonical_tactics_no_duplicates() {
        // Pin: CANONICAL_TACTICS has no duplicates so the
        // tactic-recognition path stays linear.
        use std::collections::HashSet;
        let set: HashSet<&str> = CANONICAL_TACTICS.iter().copied().collect();
        assert_eq!(
            set.len(),
            CANONICAL_TACTICS.len(),
            "CANONICAL_TACTICS contains duplicates"
        );
    }

    // ----- AuditTrail impls -----

    #[test]
    fn memory_audit_trail_round_trips() {
        let t = MemoryAuditTrail::new();
        let event = LlmProtocolEvent::LlmInvoked {
            model_id: Text::from("m"),
            theorem: Text::from("foo"),
            prompt_hash: Text::from("p".repeat(64)),
            completion_hash: Text::from("c".repeat(64)),
            tactic_count: 2,
            elapsed_ms: 10,
            timestamp: 1234567890,
        };
        t.append(event.clone()).unwrap();
        let read = t.read_all().unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0], event);
    }

    #[test]
    fn filesystem_audit_trail_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let t = FilesystemAuditTrail::new(&path).unwrap();
        for i in 0..3 {
            t.append(LlmProtocolEvent::KernelAccepted {
                model_id: Text::from("m"),
                theorem: Text::from(format!("thm-{}", i)),
                prompt_hash: Text::from("p".repeat(64)),
                completion_hash: Text::from("c".repeat(64)),
                steps_checked: i,
                timestamp: 0,
            })
            .unwrap();
        }
        let read = t.read_all().unwrap();
        assert_eq!(read.len(), 3);
    }

    #[test]
    fn filesystem_audit_trail_handles_missing_file_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-yet.jsonl");
        let t = FilesystemAuditTrail::new(&path).unwrap();
        let read = t.read_all().unwrap();
        assert!(read.is_empty());
    }

    // ----- KernelGate — LCF fail-closed contract -----

    #[test]
    fn gate_accepts_well_formed_proposal() {
        let adapter = MockLlmAdapter::new("mock", vec!["intro", "auto"]);
        let checker = PatternKernelChecker::new();
        let trail = MemoryAuditTrail::new();
        let g = LlmGoalSummary::new("thm", "P");
        let v = KernelGate::new().run(&adapter, &checker, &g, &trail).unwrap();
        match v {
            KernelVerdict::Accepted { steps_checked } => assert_eq!(steps_checked, 2),
            other => panic!("expected Accepted, got {:?}", other),
        }
        let events = trail.read_all().unwrap();
        // LlmInvoked + KernelAccepted — exactly two events.
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].name(), "LlmInvoked");
        assert_eq!(events[1].name(), "KernelAccepted");
    }

    #[test]
    fn gate_rejects_proposal_with_bad_step_and_records_index() {
        let adapter = MockLlmAdapter::new("mock", vec!["intro", "xyz_garbage", "auto"]);
        let checker = PatternKernelChecker::new();
        let trail = MemoryAuditTrail::new();
        let g = LlmGoalSummary::new("thm", "P");
        let v = KernelGate::new().run(&adapter, &checker, &g, &trail).unwrap();
        match v {
            KernelVerdict::Rejected {
                failed_step_index,
                reason,
            } => {
                assert_eq!(failed_step_index, 1);
                assert!(reason.as_str().contains("xyz_garbage"));
            }
            other => panic!("expected Rejected, got {:?}", other),
        }
        let events = trail.read_all().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].name(), "LlmInvoked");
        assert_eq!(events[1].name(), "KernelRejected");
    }

    #[test]
    fn gate_rejects_apply_to_out_of_scope_lemma() {
        let adapter = MockLlmAdapter::new("mock", vec!["apply nonexistent_lemma"]);
        let checker = PatternKernelChecker::new();
        let trail = MemoryAuditTrail::new();
        let g = LlmGoalSummary::new("thm", "P");
        let v = KernelGate::new().run(&adapter, &checker, &g, &trail).unwrap();
        assert!(!v.is_accepted());
    }

    #[test]
    fn gate_records_protocol_error_on_adapter_failure() {
        // Failing adapter that always errors.
        #[derive(Debug)]
        struct FailingAdapter;
        impl LlmTacticAdapter for FailingAdapter {
            fn model_id(&self) -> Text {
                Text::from("failing")
            }
            fn propose(&self, _: &LlmGoalSummary) -> Result<LlmProofProposal, LlmError> {
                Err(LlmError::Transport(Text::from("network down")))
            }
        }
        let trail = MemoryAuditTrail::new();
        let g = LlmGoalSummary::new("thm", "P");
        let r = KernelGate::new().run(&FailingAdapter, &PatternKernelChecker::new(), &g, &trail);
        assert!(matches!(r, Err(LlmError::Transport(_))));
        let events = trail.read_all().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name(), "ProtocolError");
    }

    // ----- Acceptance criteria pin -----

    #[test]
    fn task_77_lcf_fail_closed_contract_holds() {
        // §3 of acceptance: the LLM never short-circuits the kernel.
        // Any garbage step → KernelRejected, no Accepted verdict.
        let adapter = MockLlmAdapter::new(
            "evil",
            vec!["totally bogus syntax that should fail"],
        );
        let checker = PatternKernelChecker::new();
        let trail = MemoryAuditTrail::new();
        let g = LlmGoalSummary::new("thm", "P");
        let v = KernelGate::new().run(&adapter, &checker, &g, &trail).unwrap();
        assert!(!v.is_accepted(), "kernel must reject garbage proposal");
    }

    #[test]
    fn task_77_audit_trail_captures_model_identity() {
        // §5 of acceptance: every event logs model_id + prompt hash
        // + completion hash so the proof is reproducible.
        let adapter = MockLlmAdapter::new("local/llama-3-8b-q4", vec!["intro"]);
        let trail = MemoryAuditTrail::new();
        let g = LlmGoalSummary::new("thm", "P");
        KernelGate::new()
            .run(&adapter, &PatternKernelChecker::new(), &g, &trail)
            .unwrap();
        let events = trail.read_all().unwrap();
        for e in &events {
            // model_id, prompt_hash, completion_hash all present.
            let s = serde_json::to_string(e).unwrap();
            assert!(s.contains("local/llama-3-8b-q4"));
        }
    }

    // =========================================================================
    // KernelRejectReason / KernelInferChecker (#90 hardening)
    // =========================================================================

    use verum_kernel::{AxiomRegistry, CoreTerm, FrameworkId};

    fn registry_with(names: &[&str]) -> AxiomRegistry {
        let mut reg = AxiomRegistry::new();
        for n in names {
            // Test fixture: the kernel-attestation contract is
            // *name resolution*, not subsingleton enforcement —
            // use the legacy unchecked entry point so we can
            // register names with arbitrary placeholder types.
            let ty = CoreTerm::Var(verum_common::Text::from("Bool"));
            let fw = FrameworkId {
                framework: verum_common::Text::from("verum"),
                citation: verum_common::Text::from("test-fixture"),
            };
            reg.register_legacy_unchecked(verum_common::Text::from(*n), ty, fw)
                .unwrap();
        }
        reg
    }

    #[test]
    fn reject_reason_labels_are_stable() {
        assert_eq!(
            KernelRejectReason::NotInScope {
                name: Text::from("x")
            }
            .label(),
            "not-in-scope"
        );
        assert_eq!(
            KernelRejectReason::NotKernelAttested {
                name: Text::from("x")
            }
            .label(),
            "not-kernel-attested"
        );
        assert_eq!(
            KernelRejectReason::UnknownTactic {
                head: Text::from("x")
            }
            .label(),
            "unknown-tactic"
        );
        assert_eq!(
            KernelRejectReason::MalformedSyntax {
                detail: Text::from("x")
            }
            .label(),
            "malformed-syntax"
        );
        assert_eq!(
            KernelRejectReason::Other(Text::from("x")).label(),
            "other"
        );
    }

    #[test]
    fn reject_reason_render_is_human_readable() {
        let r = KernelRejectReason::NotInScope {
            name: Text::from("foo_lemma"),
        };
        let s = r.render();
        assert!(s.as_str().contains("foo_lemma"));
        assert!(s.as_str().contains("not in scope"));
    }

    #[test]
    fn pattern_checker_typed_reasons_round_trip_through_render() {
        let g = LlmGoalSummary::new("thm", "P");
        let c = PatternKernelChecker::new();
        // Out-of-scope apply ⇒ typed NotInScope.
        match c.check_step_typed(&g, "apply nope") {
            Err(KernelRejectReason::NotInScope { name }) => {
                assert_eq!(name.as_str(), "nope");
            }
            other => panic!("expected NotInScope, got {:?}", other),
        }
        // Unknown head ⇒ typed UnknownTactic.
        match c.check_step_typed(&g, "blortify x") {
            Err(KernelRejectReason::UnknownTactic { head }) => {
                assert_eq!(head.as_str(), "blortify");
            }
            other => panic!("expected UnknownTactic, got {:?}", other),
        }
        // `apply` followed by an empty argument list ⇒
        // MalformedSyntax (the prefix `"apply "` is matched, then
        // the remaining tokens are empty).  Use a comma-only suffix
        // to drive the empty-name path through `strip_prefix`.
        match c.check_step_typed(&g, "apply ,") {
            Err(KernelRejectReason::MalformedSyntax { .. }) => {}
            other => panic!("expected MalformedSyntax, got {:?}", other),
        }
    }

    #[test]
    fn kernel_infer_checker_resolves_through_kernel_registry() {
        let reg = registry_with(&["foo_lemma"]);
        let c = KernelInferChecker::new(reg);
        // The goal's textual lemma list does NOT contain `foo_lemma`
        // — the checker MUST still accept because the kernel-side
        // registry attests the name.  This is the production
        // contract: registry, not LLM-prompt-text, is authoritative.
        let g = LlmGoalSummary::new("thm", "P");
        c.check_step(&g, "apply foo_lemma").unwrap();
    }

    #[test]
    fn kernel_infer_checker_rejects_textual_only_lemma_as_unattested() {
        // Empty kernel registry; goal claims `foo_lemma` is in scope.
        // Production checker rejects this as `NotKernelAttested` —
        // an adversarial LLM can't bypass the trust boundary just by
        // forging a prompt context.
        let reg = AxiomRegistry::new();
        let c = KernelInferChecker::new(reg);
        let mut g = LlmGoalSummary::new("thm", "P");
        g.lemmas_in_scope = vec![(
            Text::from("foo_lemma"),
            Text::from("P"),
        )];
        match c.check_step_typed(&g, "apply foo_lemma") {
            Err(KernelRejectReason::NotKernelAttested { name }) => {
                assert_eq!(name.as_str(), "foo_lemma");
            }
            other => panic!("expected NotKernelAttested, got {:?}", other),
        }
    }

    #[test]
    fn kernel_infer_checker_distinguishes_in_scope_from_unattested() {
        // Three states:
        //   1. In registry             ⇒ Ok
        //   2. In textual scope only   ⇒ NotKernelAttested
        //   3. Nowhere                 ⇒ NotInScope
        let reg = registry_with(&["registered"]);
        let c = KernelInferChecker::new(reg);
        let mut g = LlmGoalSummary::new("thm", "P");
        g.lemmas_in_scope = vec![
            (Text::from("textual_only"), Text::from("Q")),
        ];

        c.check_step(&g, "apply registered").unwrap();

        match c.check_step_typed(&g, "apply textual_only") {
            Err(KernelRejectReason::NotKernelAttested { .. }) => {}
            other => panic!("expected NotKernelAttested, got {:?}", other),
        }

        match c.check_step_typed(&g, "apply random_name") {
            Err(KernelRejectReason::NotInScope { .. }) => {}
            other => panic!("expected NotInScope, got {:?}", other),
        }
    }

    #[test]
    fn kernel_infer_checker_admits_canonical_tactics() {
        let c = KernelInferChecker::new(AxiomRegistry::new());
        let g = LlmGoalSummary::new("thm", "P");
        for t in ["intro", "auto", "simp", "trivial", "smt"] {
            c.check_step(&g, t).unwrap();
        }
    }

    #[test]
    fn kernel_infer_checker_admits_comments_and_blank_lines() {
        let c = KernelInferChecker::new(AxiomRegistry::new());
        let g = LlmGoalSummary::new("thm", "P");
        c.check_step(&g, "").unwrap();
        c.check_step(&g, "   ").unwrap();
        c.check_step(&g, "// note about the next step").unwrap();
    }

    #[test]
    fn task_90_kernel_attestation_replaces_textual_resolution() {
        // Pin the #90 hardening contract:
        //
        // The production checker is built on a kernel-side
        // `AxiomRegistry`; an `apply NAME` step succeeds only when
        // `NAME` is registered.  The textual `lemmas_in_scope`
        // view (which the LLM's prompt builder controls) is no
        // longer the trust boundary for citation resolution.
        let trusted = registry_with(&["legit_axiom"]);
        let c = KernelInferChecker::new(trusted);

        // Goal claims a different name is in scope; the LLM
        // attempts to apply it.  Production mode rejects.
        let mut g = LlmGoalSummary::new("thm", "P");
        g.lemmas_in_scope = vec![(
            Text::from("forged_axiom"),
            Text::from("anything"),
        )];
        assert!(c.check_step(&g, "apply forged_axiom").is_err());

        // Same goal, applied through the trusted name: accept.
        assert!(c.check_step(&g, "apply legit_axiom").is_ok());
    }
}
