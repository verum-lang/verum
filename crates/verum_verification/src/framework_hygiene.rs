//! Framework-hygiene pass — Rust port of the discipline rules from
//! `core/meta/framework_hygiene.vr`. The Verum stdlib defines R1
//! (foundation-neutral names), R2 (ε-coordinate canonicalisable),
//! and R3 (meta-classifier uniqueness) as helper functions but the
//! compiler never invokes them — they're orphan stdlib code in the
//! same way the K-rules were orphan kernel code before this commit
//! series.
//!
//! This module ports R1+R2+R3 into Rust and exposes them as a
//! first-class [`HygieneRecheckPass`] so the discipline applies on
//! every `verum verify` invocation, catching stdlib-author and
//! framework-author mistakes uniformly.
//!
//! # Severity contract (mirrors the Verum stdlib spec)
//!
//!   * **R1** (foundation-neutral names) — Warning. A brand-prefix
//!     name (`diakrisis_*`, `actic_*`, `msfs_*`, `uhm_*`,
//!     `noesis_*`) embedded in a public axiom name is a hygiene
//!     concern, not a soundness one. The build can continue.
//!   * **R2** (ε-coordinate canonicalisable) — Warning. The
//!     AST-layer parser already rejects malformed `@enact(epsilon
//!     = ...)` strings; R2 catches strings that slipped through
//!     a refactor.
//!   * **R3** (meta-classifier uniqueness) — Error. Per VVA
//!     §10.4.1 only one framework may play the meta-classifier
//!     role per module-tree; a violation is a coordinate-system
//!     conflict that the build cannot recover from.
//!
//! `HygieneRecheckPass` returns `success == false` only when an
//! Error-severity diagnostic fires. Warnings are recorded for the
//! caller to surface but do not halt the pipeline.

use crate::context::VerificationContext;
use crate::level::VerificationLevel;
use crate::passes::{VerificationError, VerificationPass, VerificationResult};
use std::time::Instant;
use verum_ast::attr::Attribute;
use verum_ast::decl::ItemKind;
use verum_ast::Module;
use verum_common::{List, Text};

// =============================================================================
// Diagnostic types
// =============================================================================

/// Severity of a hygiene-rule diagnostic. Mirrors the
/// `HygieneSeverity` enum from `core/meta/framework_hygiene.vr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HygieneSeverity {
    Info,
    Warning,
    Error,
}

impl HygieneSeverity {
    /// Stable lower-case label for diagnostic surfaces.
    pub fn as_str(&self) -> &'static str {
        match self {
            HygieneSeverity::Info => "info",
            HygieneSeverity::Warning => "warning",
            HygieneSeverity::Error => "error",
        }
    }
}

/// One hygiene-rule diagnostic — one violation of R1, R2, or R3.
#[derive(Debug, Clone)]
pub struct HygieneDiagnostic {
    pub rule: &'static str,
    pub severity: HygieneSeverity,
    pub message: Text,
}

// =============================================================================
// R1 — foundation-neutral names
// =============================================================================

/// Brand-prefix denylist. Mirrors
/// `core/meta/framework_hygiene.vr::name_has_brand_prefix`.
const BRAND_PREFIXES: &[&str] = &[
    "diakrisis_",
    "actic_",
    "msfs_",
    "uhm_",
    "noesis_",
];

/// True iff the declaration name embeds a corpus brand identifier
/// at any segment-prefix position. Corpus identifiers belong in
/// the `@framework(<corpus>, "<citation>")` annotation, NOT in the
/// public-facing axiom name.
pub fn name_has_brand_prefix(name: &str) -> bool {
    BRAND_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// R1 — validate a single declaration's name. Returns `Some(diag)`
/// on a brand-prefix violation, `None` when clean.
pub fn validate_foundation_neutral_name(decl_name: &str) -> Option<HygieneDiagnostic> {
    if name_has_brand_prefix(decl_name) {
        return Some(HygieneDiagnostic {
            rule: "R1",
            severity: HygieneSeverity::Warning,
            message: Text::from(format!(
                "axiom name '{}' embeds a corpus brand identifier; \
                 use a foundation-neutral name and put the corpus citation \
                 in @framework(<corpus>, \"...\")",
                decl_name
            )),
        });
    }
    None
}

// =============================================================================
// R2 — ε-coordinate canonicalisable
// =============================================================================

/// True iff the supplied ε-coordinate string is canonicalisable
/// per the EnactAttr discipline: primitive form (`ε_*` / `epsilon_*`)
/// or ordinal form (digits, `ω`, `Ω`, `+`, `·`, parens, whitespace).
pub fn epsilon_is_canonicalisable(s: &str) -> bool {
    if s.starts_with("ε_") || s.starts_with("epsilon_") {
        return true;
    }
    s.chars().all(|c| {
        c.is_ascii_digit()
            || c == 'ω'
            || c == 'Ω'
            || c == '+'
            || c == '·'
            || c == '('
            || c == ')'
            || c.is_whitespace()
    })
}

/// R2 — validate that an ε-coordinate string is canonicalisable.
pub fn validate_epsilon_canonicalisable(epsilon: &str) -> Option<HygieneDiagnostic> {
    if epsilon_is_canonicalisable(epsilon) {
        None
    } else {
        Some(HygieneDiagnostic {
            rule: "R2",
            severity: HygieneSeverity::Warning,
            message: Text::from(format!(
                "ε-coordinate '{}' is not canonicalisable; expected primitive name \
                 (ε_math, …) or ordinal (ω, ω+1, ω·2, …)",
                epsilon
            )),
        })
    }
}

// =============================================================================
// R3 — meta-classifier uniqueness
// =============================================================================

/// R3 — at most one framework may play the meta-classifier role
/// per module-tree. Takes the list of distinct framework-corpus
/// names that appear in `@framework(<corpus>, ...)` annotations
/// AND ship ≥ 5 axioms (the structural meta-classifier signature
/// per `framework_hygiene.vr`); returns Error when more than one
/// candidate is found.
pub fn validate_meta_classifier_uniqueness(
    candidates: &[Text],
) -> Option<HygieneDiagnostic> {
    if candidates.len() <= 1 {
        return None;
    }
    let names: Vec<&str> = candidates.iter().map(|t| t.as_str()).collect();
    Some(HygieneDiagnostic {
        rule: "R3",
        severity: HygieneSeverity::Error,
        message: Text::from(format!(
            "multiple meta-classifier frameworks detected: {}. Per VVA §10.4.1 only one \
             framework may play the meta-classifier role at a time; the others must be \
             coordinate-point frameworks.",
            names.join(", ")
        )),
    })
}

// =============================================================================
// Attribute helpers
// =============================================================================

/// Extract the corpus name from an `@framework(<corpus>, "<citation>")`
/// attribute. The corpus appears as the first positional argument,
/// which is normally a Path expression naming the corpus identifier.
pub fn framework_corpus(attr: &Attribute) -> Option<Text> {
    if attr.name.as_str() != "framework" {
        return None;
    }
    use verum_ast::expr::ExprKind;
    use verum_ast::ty::PathSegment;
    let args = match &attr.args {
        verum_common::Maybe::Some(a) => a,
        verum_common::Maybe::None => return None,
    };
    let first = args.first()?;
    match &first.kind {
        ExprKind::Path(path) => {
            let last = path.segments.last()?;
            match last {
                PathSegment::Name(ident) => Some(ident.name.clone()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Extract the ε-coordinate string from an `@enact(epsilon = "<...>")`
/// attribute. The coordinate appears as a `epsilon = "..."` named
/// argument or — the simplified path the AST surfaces — a Text
/// literal in the argument list.
pub fn enact_epsilon(attr: &Attribute) -> Option<Text> {
    if attr.name.as_str() != "enact" {
        return None;
    }
    use verum_ast::expr::ExprKind;
    use verum_ast::literal::LiteralKind;
    let args = match &attr.args {
        verum_common::Maybe::Some(a) => a,
        verum_common::Maybe::None => return None,
    };
    for arg in args.iter() {
        if let ExprKind::Literal(lit) = &arg.kind {
            if let LiteralKind::Text(t) = &lit.kind {
                return Some(Text::from(t.as_str()));
            }
        }
    }
    None
}

// =============================================================================
// Item walker — collects (name, attributes) pairs by ItemKind
// =============================================================================

/// Pull the (decl-name, attribute-list) pair out of an Item kind
/// for the variants that can carry @framework / @enact. Returns
/// None when the item kind doesn't expose attributes (Mount /
/// Module / FFIBoundary etc.).
fn item_name_and_attrs<'a>(
    kind: &'a ItemKind,
) -> Option<(Text, &'a List<Attribute>)> {
    // Const/Static decls don't expose attributes in the current
    // AST, so they aren't covered by R1/R2 — those rules apply to
    // declarations that can carry @framework / @enact, which in
    // practice are Function / Type / Theorem / Lemma / Corollary /
    // Axiom decls.
    match kind {
        ItemKind::Function(f) => Some((f.name.name.clone(), &f.attributes)),
        ItemKind::Type(t) => Some((t.name.name.clone(), &t.attributes)),
        ItemKind::Theorem(d) | ItemKind::Lemma(d) | ItemKind::Corollary(d) => {
            Some((d.name.name.clone(), &d.attributes))
        }
        ItemKind::Axiom(a) => Some((a.name.name.clone(), &a.attributes)),
        _ => None,
    }
}

// =============================================================================
// HygieneRecheckPass
// =============================================================================

/// First-class verification pass that runs framework-hygiene rules
/// R1+R2+R3 on every module. Inserted into the default pipeline
/// after `KernelRecheckPass`.
#[derive(Debug)]
pub struct HygieneRecheckPass {
    diagnostics: Vec<HygieneDiagnostic>,
    /// R3 threshold — minimum @framework-annotated declaration
    /// count for a corpus to qualify as a meta-classifier
    /// candidate. Default 5 (matches the structural signature
    /// from `core/meta/framework_hygiene.vr`); configurable per
    /// corpus per #203 (e.g., owl2_fs ships 64 axioms vs custom
    /// user packages with 2-3).
    meta_classifier_threshold: usize,
    /// V8 (#209, B8) — designated meta-classifier corpus, exempt
    /// from R3 threshold counting. VVA §10.4.1 mandates exactly
    /// one corpus play the meta-classifier role; the designation
    /// is a project-level configuration item, not derived from
    /// axiom count alone. Pre-V8 the designated corpus was
    /// itself counted as a candidate and could falsely participate
    /// in the "multiple meta-classifier frameworks detected" R3
    /// error message, even though it's the legitimate one. The
    /// fix: filter the designated corpus out of the candidate
    /// list before applying the |candidates| >= 2 violation rule.
    /// `None` means no corpus is officially designated — falls
    /// back to pre-V8 behaviour (R3 fires on any |candidates|
    /// >= 2).
    designated_meta_classifier: Option<Text>,
}

/// Default R3 threshold per VVA §13 / `framework_hygiene.vr`.
pub const DEFAULT_META_CLASSIFIER_THRESHOLD: usize = 5;

impl Default for HygieneRecheckPass {
    fn default() -> Self {
        Self::new()
    }
}

impl HygieneRecheckPass {
    pub fn new() -> Self {
        Self {
            diagnostics: Vec::new(),
            meta_classifier_threshold: DEFAULT_META_CLASSIFIER_THRESHOLD,
            designated_meta_classifier: None,
        }
    }

    /// Construct with a custom R3 meta-classifier threshold.
    /// Used by per-corpus configurations where the default 5
    /// is too low (lurie_htt ships hundreds of axioms) or too
    /// high (custom user packages with 2-3 axioms might still
    /// want strict R3 enforcement).
    pub fn with_meta_classifier_threshold(threshold: usize) -> Self {
        Self {
            diagnostics: Vec::new(),
            meta_classifier_threshold: threshold,
            designated_meta_classifier: None,
        }
    }

    /// V8 (#209, B8) — set the designated meta-classifier corpus.
    /// That corpus is exempt from R3 threshold counting per VVA
    /// §10.4.1: the designated corpus is *expected* to ship many
    /// foundational axioms; only undesignated corpora reaching
    /// the threshold count as candidates for the violation.
    pub fn with_designated_meta_classifier(mut self, corpus: Text) -> Self {
        self.designated_meta_classifier = Some(corpus);
        self
    }

    /// The R3 threshold currently configured.
    pub fn meta_classifier_threshold(&self) -> usize {
        self.meta_classifier_threshold
    }

    /// V8 (#209, B8) — name of the designated meta-classifier
    /// corpus, if any. Returns `None` when no designation is
    /// set (pre-V8 fallback behaviour).
    pub fn designated_meta_classifier(&self) -> Option<&Text> {
        self.designated_meta_classifier.as_ref()
    }

    /// All diagnostics (Warning + Error) accumulated by the most
    /// recent `run`.
    pub fn diagnostics(&self) -> &[HygieneDiagnostic] {
        &self.diagnostics
    }

    /// Count of Error-severity diagnostics — the value
    /// `result.success` keys off.
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == HygieneSeverity::Error)
            .count()
    }

    /// Check one ItemKind for R1+R2 + accumulate its corpus into
    /// the meta-classifier counter. Used by both the top-level
    /// item walker and the V5 impl-block recursion.
    fn check_one_item_kind(
        &mut self,
        kind: &ItemKind,
        framework_corpus_axiom_count: &mut std::collections::HashMap<Text, usize>,
    ) {
        let (name, attrs) = match item_name_and_attrs(kind) {
            Some(p) => p,
            None => return,
        };
        let mut item_has_framework = false;
        for attr in attrs.iter() {
            if let Some(corpus) = framework_corpus(attr) {
                item_has_framework = true;
                *framework_corpus_axiom_count.entry(corpus).or_insert(0) += 1;
            }
            if let Some(epsilon) = enact_epsilon(attr) {
                if let Some(d) = validate_epsilon_canonicalisable(epsilon.as_str()) {
                    self.diagnostics.push(d);
                }
            }
        }
        // R1 fires only on items that *carry* a @framework
        // annotation — the rule is about framework-author
        // hygiene, not user-code hygiene.
        if item_has_framework {
            if let Some(d) = validate_foundation_neutral_name(name.as_str()) {
                self.diagnostics.push(d);
            }
        }
    }
}

impl VerificationPass for HygieneRecheckPass {
    fn run(
        &mut self,
        module: &Module,
        _ctx: &mut VerificationContext,
    ) -> Result<VerificationResult, VerificationError> {
        let start = Instant::now();
        self.diagnostics.clear();

        let mut meta_classifier_candidates: Vec<Text> = Vec::new();
        let mut framework_corpus_axiom_count: std::collections::HashMap<Text, usize> =
            std::collections::HashMap::new();

        for item in &module.items {
            self.check_one_item_kind(
                &item.kind,
                &mut framework_corpus_axiom_count,
            );
            // V2 (#193) — descend into impl blocks. Methods inside
            // `implement Foo { fn method(...) { ... } }` can carry
            // @framework / @enact attributes; these were invisible
            // to the V0 walker.
            if let ItemKind::Impl(impl_decl) = &item.kind {
                for impl_item in impl_decl.items.iter() {
                    if let verum_ast::decl::ImplItemKind::Function(func) =
                        &impl_item.kind
                    {
                        // Pretend the impl-item is a top-level
                        // Function for the purposes of the
                        // hygiene walker — we want the same
                        // R1/R2/R3 treatment.
                        let inner_kind = ItemKind::Function(func.clone());
                        self.check_one_item_kind(
                            &inner_kind,
                            &mut framework_corpus_axiom_count,
                        );
                    }
                }
            }
        }

        // R3 — count meta-classifier candidates per VVA §10.4.1.
        // A corpus qualifies as a meta-classifier candidate when
        // it ships ≥ `meta_classifier_threshold` framework-
        // annotated declarations (default 5, configurable per
        // corpus per #203).
        //
        // V8 (#209, B8) — the designated meta-classifier corpus
        // is exempt from this counting. VVA §10.4.1 names the
        // designated corpus as the legitimate occupant of the
        // meta-classifier role; treating it as a "candidate"
        // would let R3 fire against the corpus that's allowed
        // to ship many axioms by design.
        for (corpus, count) in framework_corpus_axiom_count.iter() {
            if *count < self.meta_classifier_threshold {
                continue;
            }
            if let Some(designated) = &self.designated_meta_classifier {
                if corpus.as_str() == designated.as_str() {
                    continue;
                }
            }
            meta_classifier_candidates.push(corpus.clone());
        }

        // R4 (#197 V0) — framework-compatibility audit. Walk every
        // (distinct) corpus appearing in @framework annotations
        // through the well-known incompatibility matrix. Each
        // match surfaces an Error-severity diagnostic; the
        // pipeline fail-fast contract halts the build (a known-
        // inconsistent axiom bundle derives False, breaking every
        // downstream theorem).
        let all_corpora: Vec<Text> = framework_corpus_axiom_count
            .keys()
            .cloned()
            .collect();
        let conflicts = crate::framework_compat::audit_framework_set(&all_corpora);
        for d in conflicts {
            self.diagnostics.push(d);
        }
        if let Some(d) = validate_meta_classifier_uniqueness(&meta_classifier_candidates) {
            self.diagnostics.push(d);
        }

        let success = self.error_count() == 0;
        let result = if success {
            VerificationResult::success(VerificationLevel::Runtime, start.elapsed(), List::new())
        } else {
            VerificationResult::failure(VerificationLevel::Runtime, start.elapsed())
        };
        Ok(result)
    }

    fn name(&self) -> &str {
        "framework_hygiene"
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_prefixes_detected() {
        assert!(name_has_brand_prefix("diakrisis_classify"));
        assert!(name_has_brand_prefix("actic_lift"));
        assert!(name_has_brand_prefix("msfs_coord"));
        assert!(name_has_brand_prefix("uhm_translate"));
        assert!(name_has_brand_prefix("noesis_step"));
    }

    #[test]
    fn clean_names_pass_r1() {
        assert!(!name_has_brand_prefix("articulation_enactment_morita_duality"));
        assert!(!name_has_brand_prefix("classify"));
        assert!(!name_has_brand_prefix("translate"));
        // Substring (not prefix) is fine — only prefix matches.
        assert!(!name_has_brand_prefix("user_diakrisis_helper"));
    }

    #[test]
    fn epsilon_primitive_form_canonicalisable() {
        assert!(epsilon_is_canonicalisable("ε_math"));
        assert!(epsilon_is_canonicalisable("ε_compute"));
        assert!(epsilon_is_canonicalisable("ε_classify"));
        assert!(epsilon_is_canonicalisable("epsilon_math"));
    }

    #[test]
    fn epsilon_ordinal_form_canonicalisable() {
        assert!(epsilon_is_canonicalisable("0"));
        assert!(epsilon_is_canonicalisable("ω"));
        assert!(epsilon_is_canonicalisable("ω+1"));
        assert!(epsilon_is_canonicalisable("ω·2"));
        assert!(epsilon_is_canonicalisable("ω·2+1"));
        assert!(epsilon_is_canonicalisable("Ω"));
    }

    #[test]
    fn epsilon_garbage_rejected() {
        assert!(!epsilon_is_canonicalisable("foo_bar"));
        assert!(!epsilon_is_canonicalisable("definitely not an ordinal"));
    }

    #[test]
    fn r3_passes_for_zero_or_one_candidates() {
        assert!(validate_meta_classifier_uniqueness(&[]).is_none());
        assert!(validate_meta_classifier_uniqueness(&[Text::from("diakrisis")]).is_none());
    }

    #[test]
    fn r3_errors_on_multiple_candidates() {
        let cs = vec![Text::from("diakrisis"), Text::from("actic")];
        let d = validate_meta_classifier_uniqueness(&cs).expect("R3 must error");
        assert_eq!(d.severity, HygieneSeverity::Error);
        assert_eq!(d.rule, "R3");
        assert!(d.message.as_str().contains("diakrisis"));
        assert!(d.message.as_str().contains("actic"));
    }

    #[test]
    fn r1_diagnostic_well_formed() {
        let d = validate_foundation_neutral_name("diakrisis_step").expect("R1 must fire");
        assert_eq!(d.severity, HygieneSeverity::Warning);
        assert_eq!(d.rule, "R1");
        assert!(d.message.as_str().contains("diakrisis_step"));
    }

    #[test]
    fn severity_as_str_stable() {
        assert_eq!(HygieneSeverity::Info.as_str(), "info");
        assert_eq!(HygieneSeverity::Warning.as_str(), "warning");
        assert_eq!(HygieneSeverity::Error.as_str(), "error");
    }

    #[test]
    fn default_meta_classifier_threshold_is_five() {
        assert_eq!(DEFAULT_META_CLASSIFIER_THRESHOLD, 5);
    }

    #[test]
    fn pass_uses_default_threshold_unless_overridden() {
        let p = HygieneRecheckPass::new();
        assert_eq!(p.meta_classifier_threshold(), DEFAULT_META_CLASSIFIER_THRESHOLD);
    }

    #[test]
    fn pass_with_custom_threshold_records_choice() {
        let p = HygieneRecheckPass::with_meta_classifier_threshold(10);
        assert_eq!(p.meta_classifier_threshold(), 10);
    }
}
