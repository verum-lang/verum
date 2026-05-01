//! Framework-citation manifest.
//!

//! Verum's trust extension via `@framework(<system>, "<path>")`
//! attributes is the mechanism by which axioms cite upstream
//! verified proofs (mathlib4, Coq stdlib, ZFC, …). This module
//! walks AST items, extracts every framework citation, and emits a
//! structured manifest that audit gates and CI pipelines can
//! consume:
//!

//!  - Audit dashboards: enumerate per-system citation counts.
//!  - Path verification: for each citation, shell out to the
//!  upstream toolchain to verify the path exists.
//!  - Drift detection: compare a manifest snapshot to the current
//!  citations to flag any silent change in the trust extension.
//!

//! The manifest is the data layer underneath
//! `verum audit --framework-axioms` and
//! `verum audit --soundness-iou`'s `DischargedByFramework` rows.
//!

//! ## Schema
//!

//! ```json
//! {
//!  "rows": [
//!  {
//!  "decl_name": "church_rosser_confluence",
//!  "decl_kind": "theorem",
//!  "framework": "mathlib4",
//!  "citation": "Mathlib.Computability.Lambda.ChurchRosser"
//!  },
//!  ...
//!  ],
//!  "by_framework": { "mathlib4": 7, "zfc": 1, "coq_stdlib": 1, ... }
//! }
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use verum_ast::Literal;
use verum_ast::LiteralKind;
use verum_ast::decl::{Item, ItemKind};
use verum_ast::expr::ExprKind;

/// One framework citation: a `@framework(<framework>, "<citation>")`
/// attribute on a theorem / lemma / corollary / axiom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkCitation {
    /// Name of the declaration carrying the citation.
    pub decl_name: String,
    /// Diagnostic kind tag — `"theorem"`, `"lemma"`, `"corollary"`,
    /// `"axiom"`, etc.
    pub decl_kind: String,
    /// Framework name as written in source (`"mathlib4"`, `"zfc"`,
    /// `"coq_stdlib"`, …). Matches the canonical
    /// [`crate::foreign_system::ForeignSystem::framework_tag`] when
    /// the citation refers to a supported foreign system.
    pub framework: String,
    /// Citation path / location string. For mathlib4 citations
    /// this is typically a dotted module path (`"Mathlib.Computability.
    /// Lambda.ChurchRosser"`); for ZFC citations it's a free-form
    /// description.
    pub citation: String,
}

/// Aggregate framework-citation manifest for a module / project /
/// directory walk.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FrameworkCitationManifest {
    /// All citations in source order.
    pub rows: Vec<FrameworkCitation>,
    /// Per-framework count. Keys are framework names
    /// (`"mathlib4"`, `"zfc"`, …); values are citation counts.
    pub by_framework: BTreeMap<String, usize>,
}

impl FrameworkCitationManifest {
    /// Empty manifest — initial accumulator state.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Total citation count across all frameworks.
    pub fn total(&self) -> usize {
        self.rows.len()
    }

    /// Citations from a specific framework (filter helper).
    pub fn for_framework(&self, framework: &str) -> Vec<&FrameworkCitation> {
        self.rows
            .iter()
            .filter(|r| r.framework == framework)
            .collect()
    }

    /// All distinct framework names that appear in this manifest.
    pub fn frameworks(&self) -> Vec<String> {
        self.by_framework.keys().cloned().collect()
    }
}

/// **Walk a Verum module's items and collect every
/// `@framework(<system>, "<citation>")` attribute.**
///

/// Inspects:
///  - Theorem / Lemma / Corollary attributes (declaration-level).
///  - Axiom attributes.
///  - Item-level attributes (which the AST also surfaces as
///  `Item.attributes`).
///

/// The collector handles both:
///  - 2-arg form: `@framework(<framework>, "<citation>")`
///  (the canonical shape).
///  - 1-arg form: `@framework("<citation>")` (legacy / shorthand —
///  framework defaults to `"unknown"`).
///

/// Citations whose framework or path can't be parsed (malformed
/// attributes) are silently skipped; the audit gate's separate
/// validation pass flags malformed attributes via
/// `audit_framework_axioms`.
pub fn collect_framework_citations(items: &[Item]) -> FrameworkCitationManifest {
    let mut manifest = FrameworkCitationManifest::empty();
    for item in items {
        let (decl_name, decl_kind, decl_attrs) = match &item.kind {
            ItemKind::Theorem(t) => (
                t.name.name.to_string(),
                "theorem",
                Some(t.attributes.iter().collect::<Vec<_>>()),
            ),
            ItemKind::Lemma(t) => (
                t.name.name.to_string(),
                "lemma",
                Some(t.attributes.iter().collect::<Vec<_>>()),
            ),
            ItemKind::Corollary(t) => (
                t.name.name.to_string(),
                "corollary",
                Some(t.attributes.iter().collect::<Vec<_>>()),
            ),
            ItemKind::Axiom(a) => (
                a.name.name.to_string(),
                "axiom",
                Some(a.attributes.iter().collect::<Vec<_>>()),
            ),
            _ => continue,
        };
        let mut all_attrs: Vec<&verum_ast::Attribute> = item.attributes.iter().collect();
        if let Some(more) = decl_attrs {
            all_attrs.extend(more);
        }
        for attr in all_attrs {
            if !attr.is_named("framework") {
                continue;
            }
            if let Some(citation) = parse_framework_attr(attr) {
                let row = FrameworkCitation {
                    decl_name: decl_name.clone(),
                    decl_kind: decl_kind.to_string(),
                    framework: citation.0,
                    citation: citation.1,
                };
                *manifest
                    .by_framework
                    .entry(row.framework.clone())
                    .or_insert(0) += 1;
                manifest.rows.push(row);
            }
        }
    }
    manifest
}

/// Parse a `@framework(...)` attribute into `(framework, citation)`.
/// Returns `None` for malformed shapes.
fn parse_framework_attr(attr: &verum_ast::Attribute) -> Option<(String, String)> {
    use verum_common::Maybe;
    let args = match &attr.args {
        Maybe::Some(args) => args,
        Maybe::None => return None,
    };
    let arg_vec: Vec<_> = args.iter().collect();
    match arg_vec.len() {
        2 => {
            // @framework(<ident>, "<string>")
            let framework = ident_or_path(&arg_vec[0].kind)?;
            let citation = string_literal(&arg_vec[1].kind)?;
            Some((framework, citation))
        }
        1 => {
            // @framework("<string>") — legacy shorthand
            let citation = string_literal(&arg_vec[0].kind)?;
            Some(("unknown".to_string(), citation))
        }
        _ => None,
    }
}

/// Extract a string literal value from an expression.
fn string_literal(kind: &ExprKind) -> Option<String> {
    if let ExprKind::Literal(lit) = kind {
        if let LiteralKind::Text(s) = &lit.kind {
            return Some(s.as_str().to_string());
        }
    }
    None
}

/// Extract an identifier or single-segment path name.
fn ident_or_path(kind: &ExprKind) -> Option<String> {
    use verum_ast::ty::PathSegment;
    match kind {
        ExprKind::Path(path) => {
            if path.segments.len() == 1 {
                if let PathSegment::Name(ident) = &path.segments[0] {
                    return Some(ident.name.to_string());
                }
            }
            None
        }
        ExprKind::Literal(lit) => {
            if let LiteralKind::Text(s) = &lit.kind {
                Some(s.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

// `verum_ast::Literal` is the typed-literal form; we only need the
// kind discriminator above.
#[allow(unused_imports)]
fn _ensure_literal_in_scope(_: Literal) {}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::decl::{AxiomDecl, Item, TheoremDecl};
    use verum_ast::ty::Ident as TyIdent;
    use verum_ast::{Attribute, Expr};
    use verum_common::span::FileId;
    use verum_common::{List, Maybe, Span};

    fn span() -> Span {
        Span::dummy()
    }

    fn string_lit_expr(s: &str) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::string(verum_common::Text::from(s), span())),
            span(),
        )
    }

    fn path_expr(name: &str) -> Expr {
        let mut list = List::new();
        list.push(verum_ast::ty::PathSegment::Name(TyIdent {
            name: name.into(),
            span: span(),
        }));
        let path = verum_ast::ty::Path::new(list, span());
        Expr::new(ExprKind::Path(path), span())
    }

    fn make_framework_attr(framework: &str, citation: &str) -> Attribute {
        let mut args = List::new();
        args.push(path_expr(framework));
        args.push(string_lit_expr(citation));
        Attribute::new("framework".into(), Maybe::Some(args), span())
    }

    fn make_theorem_with_attrs(name: &str, attrs: Vec<Attribute>) -> Item {
        let mut t = TheoremDecl::new(
            verum_ast::ty::Ident {
                name: name.into(),
                span: span(),
            },
            string_lit_expr("dummy"),
            span(),
        );
        let mut list = List::new();
        for a in attrs {
            list.push(a);
        }
        t.attributes = list;
        Item::new(ItemKind::Theorem(t), span())
    }

    fn make_axiom_with_attrs(name: &str, attrs: Vec<Attribute>) -> Item {
        let mut a = AxiomDecl::new(
            verum_ast::ty::Ident {
                name: name.into(),
                span: span(),
            },
            string_lit_expr("dummy"),
            span(),
        );
        let mut list = List::new();
        for at in attrs {
            list.push(at);
        }
        a.attributes = list;
        Item::new(ItemKind::Axiom(a), span())
    }

    #[test]
    fn collect_framework_citations_two_arg_form() {
        let items = vec![make_theorem_with_attrs(
            "church_rosser",
            vec![make_framework_attr(
                "mathlib4",
                "Mathlib.Computability.Lambda.ChurchRosser",
            )],
        )];
        let manifest = collect_framework_citations(&items);
        assert_eq!(manifest.total(), 1);
        assert_eq!(manifest.rows[0].decl_name, "church_rosser");
        assert_eq!(manifest.rows[0].decl_kind, "theorem");
        assert_eq!(manifest.rows[0].framework, "mathlib4");
        assert_eq!(
            manifest.rows[0].citation,
            "Mathlib.Computability.Lambda.ChurchRosser",
        );
        assert_eq!(manifest.by_framework.get("mathlib4"), Some(&1));
    }

    #[test]
    fn collect_framework_citations_groups_by_framework() {
        let items = vec![
            make_theorem_with_attrs("thm_a", vec![make_framework_attr("mathlib4", "Mathlib.A")]),
            make_theorem_with_attrs("thm_b", vec![make_framework_attr("mathlib4", "Mathlib.B")]),
            make_theorem_with_attrs("thm_c", vec![make_framework_attr("zfc", "Foundation")]),
        ];
        let manifest = collect_framework_citations(&items);
        assert_eq!(manifest.total(), 3);
        assert_eq!(manifest.by_framework.get("mathlib4"), Some(&2));
        assert_eq!(manifest.by_framework.get("zfc"), Some(&1));
    }

    #[test]
    fn collect_framework_citations_handles_axioms() {
        let items = vec![make_axiom_with_attrs(
            "trusted_axiom",
            vec![make_framework_attr("zfc", "axiom-of-choice")],
        )];
        let manifest = collect_framework_citations(&items);
        assert_eq!(manifest.rows[0].decl_kind, "axiom");
        assert_eq!(manifest.rows[0].framework, "zfc");
    }

    #[test]
    fn collect_framework_citations_skips_unrelated_attributes() {
        let items = vec![make_theorem_with_attrs(
            "irrelevant",
            vec![Attribute::new("verify".into(), Maybe::None, span())],
        )];
        let manifest = collect_framework_citations(&items);
        assert_eq!(manifest.total(), 0);
    }

    #[test]
    fn collect_framework_citations_handles_malformed_silent() {
        // Zero args — malformed. Should be silently skipped (the
        // separate audit pass flags malformed attrs).
        let items = vec![make_theorem_with_attrs(
            "malformed",
            vec![Attribute::new("framework".into(), Maybe::None, span())],
        )];
        let manifest = collect_framework_citations(&items);
        assert_eq!(manifest.total(), 0);
    }

    #[test]
    fn manifest_serde_round_trip() {
        let items = vec![make_theorem_with_attrs(
            "thm",
            vec![make_framework_attr("mathlib4", "Mathlib.X")],
        )];
        let manifest = collect_framework_citations(&items);
        let json = serde_json::to_string(&manifest).unwrap();
        let restored: FrameworkCitationManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, manifest);
    }

    #[test]
    fn manifest_for_framework_filters() {
        let items = vec![
            make_theorem_with_attrs("a", vec![make_framework_attr("mathlib4", "Mathlib.A")]),
            make_theorem_with_attrs("b", vec![make_framework_attr("zfc", "F1")]),
            make_theorem_with_attrs("c", vec![make_framework_attr("mathlib4", "Mathlib.C")]),
        ];
        let manifest = collect_framework_citations(&items);
        let mathlib_rows = manifest.for_framework("mathlib4");
        assert_eq!(mathlib_rows.len(), 2);
        let names: Vec<&str> = mathlib_rows.iter().map(|r| r.decl_name.as_str()).collect();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"c"));
    }

    #[test]
    fn manifest_frameworks_returns_distinct() {
        let items = vec![
            make_theorem_with_attrs("a", vec![make_framework_attr("mathlib4", "Mathlib.A")]),
            make_theorem_with_attrs("b", vec![make_framework_attr("mathlib4", "Mathlib.B")]),
            make_theorem_with_attrs("c", vec![make_framework_attr("zfc", "F")]),
        ];
        let manifest = collect_framework_citations(&items);
        let frameworks = manifest.frameworks();
        assert_eq!(frameworks.len(), 2);
        assert!(frameworks.contains(&"mathlib4".to_string()));
        assert!(frameworks.contains(&"zfc".to_string()));
    }

    #[test]
    fn _file_id_smoke() {
        // Trivial pin so unused-import lint doesn't fire if the
        // import gets reorganised.
        let _ = FileId::new(0);
    }
}
